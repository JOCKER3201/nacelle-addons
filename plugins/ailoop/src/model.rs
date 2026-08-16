//! What the panel KNOWS: the path in the box, the run in flight, and
//! what the daemon has said about it so far.
//!
//! No drawing, no theme, no socket — the daemon's events arrive as
//! [`Event`] values a caller already parsed, and every answer this
//! module gives is a state a frame can render. That is what makes "what
//! does this event do to the widget" testable without a daemon, which
//! is the test this widget most needs: the daemon is the flaky half of
//! the pair, and the mapping must hold whether or not it is there.
//!
//! # The field is not written here
//!
//! The path is an [`InputModel`] from the toolkit, and every key that
//! reaches it goes through the toolkit's own [`key_msg`] — caret motion
//! by grapheme, word motion, undo grouping, the selection and the IME
//! contract are all decided in one place for the whole project. What
//! this file adds is the meaning of the two keys the field answers with
//! an intent: Enter starts the run, Escape empties the box.

use nacelle::focus::KeyEv;
use nacelle::object::text_input::{key_msg, InputEdited, InputModel};
use nacelle_ai_client::Event;
use serde_json::Value;
use std::collections::VecDeque;

/// The longest path the box will hold, in characters. Load bearing
/// rather than tidy, exactly as the search box's cap is: the field view
/// records a caret stop per character every frame, each measured from
/// the start of the value, so drawing cost grows with the SQUARE of the
/// length. Longer than any path a person drops in a box; shorter than a
/// paste that would freeze a desktop.
pub const MAX_PATH: usize = 1024;

/// Progress lines kept. The panel is a status strip, not a terminal:
/// what matters is the last thing the daemon said, plus enough history
/// to see how it got there. Older lines fall off the front.
pub const LOG_KEEP: usize = 16;

/// What the panel says about a run whose daemon vanished mid-answer.
/// It names what happened and promises nothing about when the daemon
/// returns, because this file has no way of knowing.
pub const SAY_GONE: &str = "the daemon went away before it answered";

/// The two answers to an approval question, as log lines — what the
/// user did, written where the run's history is.
pub const SAY_ALLOWED: &str = "allowed";
pub const SAY_REFUSED: &str = "refused";

/// Where the panel is, between frames. One request at a time, on
/// purpose: the box holds one path and the daemon is asked about that
/// path, so a second START while one runs has nothing coherent to mean.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Nothing in flight. The field and the START button are live.
    Idle,
    /// Request `id` is with the daemon; progress lands in the log.
    Running { id: u64 },
    /// Request `id` stopped at the confidentiality line: the daemon
    /// asked, `desc` says what it wants, and ALLOW/REFUSE answer it.
    Waiting { id: u64, desc: String },
    /// Request `id` finished. `path` is the NEW file the daemon made —
    /// or `None` for a `done` that named no file, which is shown as
    /// exactly that rather than guessed at.
    Finished { id: u64, path: Option<String> },
    /// Request `id` failed, and `msg` is the daemon saying why — or
    /// [`SAY_GONE`] when what failed was the daemon itself.
    Failed { id: u64, msg: String },
}

/// What one key meant. The caller redraws on anything but [`Ignored`],
/// and [`Ignored`] is also the answer that says "this key was not mine"
/// — what keeps the host's focus chain and shortcuts working over a
/// panel that has the keyboard.
///
/// [`Ignored`]: Outcome::Ignored
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing here answers this key.
    Ignored,
    /// The field changed — text, caret or selection.
    Edited,
    /// Enter on a path the panel can act on: the caller sends the
    /// `tool` command and reports back through [`Model::started`].
    Start,
}

/// The path a `done` event's body names, read by the words a result
/// could plausibly travel under. v0 spells `done` as
/// `{"ev":"done","id":N,...}` and leaves the tail to the daemon's side
/// of the fence, so this reads what it knows and answers `None` for a
/// body that names no file — the same forbearance the client's parser
/// shows unknown events.
pub fn done_path(body: &Value) -> Option<String> {
    ["out", "path", "result"]
        .iter()
        .find_map(|k| body.get(k).and_then(Value::as_str))
        .map(str::to_owned)
}

/// The panel's model.
pub struct Model {
    /// The path field's state — the toolkit's, not this crate's.
    pub input: InputModel,
    pub phase: Phase,
    /// The run's history, oldest first: every `progress` and `delta`
    /// line the daemon sent about the request in flight.
    pub log: VecDeque<String>,
}

impl Model {
    pub fn new() -> Model {
        Model {
            input: InputModel::new().with_max_len(MAX_PATH),
            phase: Phase::Idle,
            log: VecDeque::new(),
        }
    }

    /// The path as the daemon will be given it: what is in the box,
    /// with the whitespace a paste drags along trimmed off. Trimmed
    /// HERE and not in the field, because what the user typed is theirs
    /// to see and edit exactly as typed.
    pub fn path(&self) -> &str {
        self.input.value().trim()
    }

    /// Whether START means anything right now: a path to act on, and no
    /// request already in flight. One request at a time — see [`Phase`].
    pub fn can_start(&self) -> bool {
        !self.path().is_empty()
            && !matches!(self.phase, Phase::Running { .. } | Phase::Waiting { .. })
    }

    /// The command went out under `id`. The old run's story goes with
    /// the old run: a log that mixed two requests would read as one.
    pub fn started(&mut self, id: u64) {
        self.phase = Phase::Running { id };
        self.log.clear();
    }

    /// Whether an event about request `id` is about the request in
    /// flight. Anything else — a stale id, an answer arriving after the
    /// run already closed, somebody's echo — is a stale answer to a
    /// question no longer asked, and it is stepped over exactly as the
    /// client steps over unknown lines. A finished or failed run keeps
    /// its answer on screen; a late event must not resurrect it.
    fn live(&self, id: u64) -> bool {
        matches!(
            self.phase,
            Phase::Running { id: r } | Phase::Waiting { id: r, .. } if r == id
        )
    }

    fn log_push(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        self.log.push_back(line);
        while self.log.len() > LOG_KEEP {
            self.log.pop_front();
        }
    }

    /// One event from the daemon, folded into the panel's state. The
    /// whole protocol-to-widget mapping is this match, which is why it
    /// lives in a file a test can reach without a socket.
    pub fn on_event(&mut self, ev: &Event) {
        match ev {
            // The handshake is the CLIENT's business; the panel has no
            // question it answers.
            Event::Hello { .. } => {}
            // A tool's streamed commentary and its progress notes are
            // the same thing to a status strip: the latest line.
            Event::Delta { id, text } if self.live(*id) => self.log_push(text.clone()),
            Event::Progress { id, msg } if self.live(*id) => self.log_push(msg.clone()),
            Event::Approval { id, desc } if self.live(*id) => {
                self.phase = Phase::Waiting { id: *id, desc: desc.clone() };
            }
            Event::Done { id, body } if self.live(*id) => {
                self.phase = Phase::Finished { id: *id, path: done_path(body) };
            }
            Event::Error { id, msg } if self.live(*id) => {
                self.phase = Phase::Failed { id: *id, msg: msg.clone() };
            }
            // An event about a request this panel is not in the middle
            // of — a stale id, an answer after Escape, somebody else's
            // conversation echoed back. Dropped, not died of.
            _ => {}
        }
    }

    /// The user answered the approval question. Gives back the id the
    /// answer must be sent under, or `None` when no question is open —
    /// the caller sends `approve` and the run goes back to Running,
    /// because the daemon proceeds (or winds down) either way and says
    /// which through its own next event.
    pub fn answered(&mut self, allow: bool) -> Option<u64> {
        let Phase::Waiting { id, .. } = self.phase else { return None };
        self.phase = Phase::Running { id };
        self.log_push((if allow { SAY_ALLOWED } else { SAY_REFUSED }).to_string());
        Some(id)
    }

    /// The socket went down under a run. The request died with the
    /// daemon — a fresh connection starts a fresh conversation, so
    /// nothing will ever answer it — and the panel says so instead of
    /// showing "working" forever. A panel with nothing in flight has
    /// lost nothing and stays exactly where it was.
    pub fn connection_lost(&mut self) {
        match self.phase {
            Phase::Running { id } | Phase::Waiting { id, .. } => {
                self.phase = Phase::Failed { id, msg: SAY_GONE.to_string() };
            }
            _ => {}
        }
    }

    /// One key.
    ///
    /// Everything is the field's except what the field answers with an
    /// intent: Enter asks to start, Escape empties the box. An Escape
    /// on an already-empty box is not this panel's — it belongs to
    /// whatever put the panel on screen.
    pub fn key(&mut self, ev: &KeyEv) -> Outcome {
        let Some(msg) = key_msg(ev) else { return Outcome::Ignored };
        match self.input.apply(msg) {
            InputEdited::Submit => {
                if self.can_start() {
                    Outcome::Start
                } else {
                    // Enter over an empty box, or over a run already in
                    // flight, is not a key this panel consumed.
                    Outcome::Ignored
                }
            }
            InputEdited::Cancel => {
                if self.input.value().is_empty() {
                    return Outcome::Ignored;
                }
                self.input.set_value("");
                Outcome::Edited
            }
            InputEdited::None | InputEdited::Rejected => Outcome::Ignored,
            // Edited, Moved, and the clipboard intents — the value or
            // the caret moved, so the frame is stale either way. The
            // clipboard itself is out of a plugin's reach (no ABI entry
            // hands one over), which is the search box's limit too.
            _ => Outcome::Edited,
        }
    }
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}
