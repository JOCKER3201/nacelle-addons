//! What the panel KNOWS: the path being typed, which action the
//! keyboard is on, and where the current request stands with the
//! daemon.
//!
//! No drawing, no theme, no socket and no clock — the client's events
//! arrive as values through [`Photo::on_event`] and every key through
//! [`Photo::key`], which is what makes "what does this event do to the
//! screen" testable without a window or a daemon.
//!
//! # The field is not written here
//!
//! The path is an [`InputModel`] from the toolkit, and every key that
//! reaches it goes through the toolkit's own [`key_msg`] — caret motion
//! by grapheme, undo grouping, the selection and the IME contract are
//! all decided in one place for the whole project, exactly as the
//! search panel's query box does it.
//!
//! # Honesty is the state machine
//!
//! The daemon's `photo` tool is NOT BUILT YET; it answers every request
//! with an error saying so. This model does not soften that: whatever
//! the daemon says lands in [`Phase::Said`] VERBATIM and is what the
//! panel shows. A widget that reworded "not built yet" into a spinner
//! would be a widget pretending to work, which is the one thing the
//! owner's specification forbids it to do.

use nacelle::focus::{Key, KeyEv};
use nacelle::object::text_input::{key_msg, InputEdited, InputModel};
use nacelle_ai_client::Event;

/// The actions the panel offers, in the order the list shows them, as
/// the words the wire will carry in `args.action`.
///
/// A PLACEHOLDER vocabulary, and named as one: the daemon's `photo`
/// tool declares no actions yet — it declares nothing, it is not built
/// — so these are the panel's own intended verbs, and every one of
/// them today earns the same honest answer from the daemon. When the
/// tool lands, its vocabulary replaces this list rather than joining
/// it.
pub const ACTIONS: &[&str] = &["enhance", "upscale", "denoise", "convert"];

/// The longest path the box will hold, in characters.
///
/// The cap is load bearing rather than tidy: the field view records a
/// caret position per character every frame, each measured from the
/// start of the value, so the drawing cost grows with the SQUARE of
/// the length. Longer than any sane path, shorter than a frozen frame.
pub const MAX_PATH: usize = 512;

/// What the panel says when an action is run over an empty path box.
/// Its own words, because this is the one answer that never reaches
/// the daemon: there is nothing to ask about.
pub const NEED_PATH: &str = "type the path of a photo first";

/// Where the current request stands. What the body of the panel draws
/// is exactly one of these, so the state machine IS the display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No request on the wire: the field and the action list.
    Ready,
    /// A request is out. `note` is the daemon's latest word on it —
    /// progress messages replace it, deltas grow it — and empty until
    /// the daemon says anything at all.
    Working { note: String },
    /// The daemon stopped at the confidentiality line and asked.
    /// SHOWN, never answered from here: this panel has no approve
    /// control yet, and a widget that answered for the user would be
    /// the autonomy the daemon exists to not have. Escape cancels.
    Waiting { desc: String },
    /// The daemon's final word — a `done`'s result or an `error`'s
    /// message, verbatim. Today that is "not built yet", and showing
    /// it is the panel's whole job.
    Said { text: String },
}

/// What one key or click meant. The caller redraws on anything but
/// [`Ignored`], and [`Ignored`] also says "not my key" — the answer
/// that lets the host spend it on focus navigation instead.
///
/// [`Ignored`]: Outcome::Ignored
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing here answers this input.
    Ignored,
    /// Something visible moved — caret, chosen row, phase.
    Moved,
    /// The path text changed.
    Edited,
    /// Send `ACTIONS[i]` on the current path to the daemon.
    Run(usize),
    /// Send a `cancel` for this request.
    Cancel(u64),
}

/// The panel's model.
pub struct Photo {
    /// The path field's state — the toolkit's, not this crate's.
    pub input: InputModel,
    /// Which action the keyboard is on. The list is static and never
    /// empty, so this is an index and not an option of one.
    cursor: usize,
    /// The request on the wire, while one is.
    req: Option<u64>,
    phase: Phase,
}

impl Photo {
    pub fn new() -> Photo {
        Photo {
            input: InputModel::new().with_max_len(MAX_PATH),
            cursor: 0,
            req: None,
            phase: Phase::Ready,
        }
    }

    pub fn path(&self) -> &str {
        self.input.value()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The request in flight, if one is.
    pub fn request(&self) -> Option<u64> {
        self.req
    }

    /// A request went out under this id — the caller's half of
    /// [`Outcome::Run`], called with what the client allocated.
    pub fn sent(&mut self, id: u64) {
        self.req = Some(id);
        self.phase = Phase::Working { note: String::new() };
    }

    /// The socket went away. The request died with it — the daemon's
    /// side of the conversation cannot answer any more — so the panel
    /// goes back to its list rather than showing a "working" that
    /// nothing is working on.
    pub fn connection_lost(&mut self) {
        self.req = None;
        self.phase = Phase::Ready;
    }

    /// One event from the daemon. Answers whether anything a frame
    /// draws has changed.
    ///
    /// Everything is keyed to the request THIS panel has in flight:
    /// an event for another id — a stale answer outliving a cancel,
    /// a daemon bug — changes nothing, and an event arriving while no
    /// request is out changes nothing either. `hello` is connection
    /// bookkeeping, not request state, so it lands in the same shrug.
    pub fn on_event(&mut self, ev: &Event) -> bool {
        let Some(req) = self.req else { return false };
        match ev {
            Event::Delta { id, text } if *id == req => {
                // A streamed answer grows in place. The photo tool is
                // not expected to stream, but the protocol allows it,
                // and a panel that dropped deltas would show silence
                // where the daemon was speaking.
                match &mut self.phase {
                    Phase::Working { note } => note.push_str(text),
                    _ => self.phase = Phase::Working { note: text.clone() },
                }
                true
            }
            Event::Progress { id, msg } if *id == req => {
                // A progress note REPLACES the last one: it is a
                // sentence about now, not a log.
                self.phase = Phase::Working { note: msg.clone() };
                true
            }
            Event::Approval { id, desc } if *id == req => {
                self.phase = Phase::Waiting { desc: desc.clone() };
                true
            }
            Event::Done { id, body } if *id == req => {
                self.req = None;
                self.phase = Phase::Said { text: done_text(body) };
                true
            }
            Event::Error { id, msg } if *id == req => {
                // Verbatim — today this is where "not built yet"
                // arrives, and the daemon's own words are the honest
                // display of it.
                self.req = None;
                self.phase = Phase::Said { text: msg.clone() };
                true
            }
            _ => false,
        }
    }

    /// Moves the chosen action by `delta`, clamped at both ends.
    ///
    /// Clamped rather than wrapped, and the answer at either end is
    /// [`Outcome::Ignored`]: an arrow that changed nothing has not
    /// been consumed, so a host with a focus chain can take it as the
    /// request to leave the list that it is.
    pub fn step(&mut self, delta: isize) -> Outcome {
        let last = ACTIONS.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, last) as usize;
        if next == self.cursor {
            return Outcome::Ignored;
        }
        self.cursor = next;
        Outcome::Moved
    }

    /// Runs action `i` — what a click on its row means, and what Enter
    /// means through [`Photo::key`].
    ///
    /// An action over an empty path box asks the daemon NOTHING: there
    /// is no request to make, so the panel answers for itself with
    /// [`NEED_PATH`] instead of sending a question with no subject.
    pub fn run(&mut self, i: usize) -> Outcome {
        if i >= ACTIONS.len() {
            return Outcome::Ignored;
        }
        self.cursor = i;
        if self.path().trim().is_empty() {
            self.phase = Phase::Said { text: NEED_PATH.to_string() };
            return Outcome::Moved;
        }
        Outcome::Run(i)
    }

    /// One key.
    pub fn key(&mut self, ev: &KeyEv) -> Outcome {
        match self.phase {
            Phase::Ready => self.key_ready(ev),
            _ => self.key_showing(ev),
        }
    }

    /// The list and the field, side by side: two keys are the LIST's
    /// and everything else is the field's — the search panel's rule,
    /// for the search panel's reason (a one-line field has nowhere
    /// vertical to go).
    fn key_ready(&mut self, ev: &KeyEv) -> Outcome {
        // …with one exception: an IME candidate window is walked with
        // the same two keys, and it owns them until the composition
        // commits.
        if !self.input.has_preedit() {
            match ev.key {
                Key::Up => return self.step(-1),
                Key::Down => return self.step(1),
                _ => {}
            }
        }
        let Some(msg) = key_msg(ev) else { return Outcome::Ignored };
        match self.input.apply(msg) {
            InputEdited::Edited => Outcome::Edited,
            InputEdited::Submit => self.run(self.cursor),
            InputEdited::Cancel => {
                // Escape empties the box. An Escape on an ALREADY
                // empty box is not this panel's: it belongs to
                // whatever put the panel on screen.
                if self.path().is_empty() {
                    return Outcome::Ignored;
                }
                self.input.set_value("");
                Outcome::Edited
            }
            InputEdited::Moved => Outcome::Moved,
            // The clipboard does not cross the plugin boundary; see
            // the search panel for the argument in full.
            InputEdited::CopyRequest { .. } | InputEdited::PasteRequest => Outcome::Ignored,
            InputEdited::Rejected | InputEdited::None => Outcome::Ignored,
        }
    }

    /// A message is on screen instead of the list.
    ///
    /// While the request still RUNS (working, or waiting at the
    /// approval line), Escape is the one key that means anything —
    /// cancel — and everything else is left alone rather than typed
    /// blind under a message. Once the daemon has SAID its piece the
    /// conversation is over: editing the path puts the list back, and
    /// so does Escape, without a cancel because there is nothing left
    /// to cancel.
    fn key_showing(&mut self, ev: &KeyEv) -> Outcome {
        if ev.key == Key::Escape {
            let req = self.req.take();
            self.phase = Phase::Ready;
            return match req {
                Some(id) => Outcome::Cancel(id),
                None => Outcome::Moved,
            };
        }
        if self.req.is_some() {
            return Outcome::Ignored;
        }
        // Phase::Said, request done: the field wakes back up.
        let Some(msg) = key_msg(ev) else { return Outcome::Ignored };
        match self.input.apply(msg) {
            InputEdited::Edited => {
                self.phase = Phase::Ready;
                Outcome::Edited
            }
            InputEdited::Moved => {
                self.phase = Phase::Ready;
                Outcome::Moved
            }
            // Enter under a final message is "yes, I read it": the
            // list comes back and nothing is re-run — a repeat should
            // be asked for, not defaulted into.
            InputEdited::Submit => {
                self.phase = Phase::Ready;
                Outcome::Moved
            }
            _ => Outcome::Ignored,
        }
    }
}

impl Default for Photo {
    fn default() -> Self {
        Photo::new()
    }
}

/// The sentence a `done` event reads as.
///
/// The spec spells the event `{"ev":"done","id":N,...}` and pins
/// nothing about the tail, so this looks for the two fields the
/// protocol's own examples use — a message, or an output path — and
/// falls back to the one honest word left when the daemon finished
/// silently.
fn done_text(body: &serde_json::Value) -> String {
    if let Some(msg) = body.get("msg").and_then(|v| v.as_str()) {
        return msg.to_string();
    }
    if let Some(out) = body.get("out").and_then(|v| v.as_str()) {
        // The one fact a result path carries that matters here: where
        // the new file went. Results never overwrite the input — the
        // daemon writes beside the source — so naming the path is
        // naming the work.
        return format!("wrote {out}");
    }
    "finished".to_string()
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod event_tests {
    use super::*;
    use serde_json::json;

    fn working(m: &Photo) -> &str {
        match m.phase() {
            Phase::Working { note } => note,
            other => panic!("expected Working, got {other:?}"),
        }
    }

    fn said(m: &Photo) -> &str {
        match m.phase() {
            Phase::Said { text } => text,
            other => panic!("expected Said, got {other:?}"),
        }
    }

    /// The mapping this widget exists to get right: every event kind
    /// the daemon can send about a request, onto the state the body
    /// draws.
    #[test]
    fn the_daemons_events_drive_the_phase() {
        let mut m = Photo::new();
        m.sent(7);
        assert_eq!(m.phase(), &Phase::Working { note: String::new() });
        assert_eq!(m.request(), Some(7));

        // A progress note replaces the silence, and the next one
        // replaces IT — a sentence about now, not a log.
        assert!(m.on_event(&Event::Progress { id: 7, msg: "reading exif".into() }));
        assert_eq!(working(&m), "reading exif");
        assert!(m.on_event(&Event::Progress { id: 7, msg: "resizing".into() }));
        assert_eq!(working(&m), "resizing");

        // Deltas grow in place: a streamed answer is one text.
        assert!(m.on_event(&Event::Delta { id: 7, text: "the ".into() }));
        assert!(m.on_event(&Event::Delta { id: 7, text: "answer".into() }));
        assert_eq!(working(&m), "resizingthe answer");

        // The approval line: shown, and NOT answered from here.
        assert!(m.on_event(&Event::Approval { id: 7, desc: "run ffmpeg on a.png".into() }));
        assert_eq!(m.phase(), &Phase::Waiting { desc: "run ffmpeg on a.png".into() });
        assert_eq!(m.request(), Some(7), "waiting is still in flight");

        // The daemon's final word ends the request.
        assert!(m.on_event(&Event::Error { id: 7, msg: "not built yet".into() }));
        assert_eq!(said(&m), "not built yet");
        assert_eq!(m.request(), None);
    }

    /// The one answer the daemon gives today, shown VERBATIM — the
    /// widget's whole honesty contract in one assertion. A panel that
    /// reworded this would be pretending the tool exists.
    #[test]
    fn not_built_yet_is_shown_in_the_daemons_own_words() {
        let mut m = Photo::new();
        m.sent(1);
        m.on_event(&Event::Error { id: 1, msg: "photo: not built yet".into() });
        assert_eq!(said(&m), "photo: not built yet");
    }

    /// `done` reads by the fields the protocol's examples carry, and
    /// finishes honestly when it carries neither.
    #[test]
    fn a_done_event_reads_as_its_result() {
        let mut m = Photo::new();
        m.sent(2);
        m.on_event(&Event::Done { id: 2, body: json!({"ev":"done","id":2,"msg":"4 photos enhanced"}) });
        assert_eq!(said(&m), "4 photos enhanced");

        m.sent(3);
        m.on_event(&Event::Done { id: 3, body: json!({"ev":"done","id":3,"out":"/p/a-enhanced.png"}) });
        assert_eq!(said(&m), "wrote /p/a-enhanced.png");

        m.sent(4);
        m.on_event(&Event::Done { id: 4, body: json!({"ev":"done","id":4}) });
        assert_eq!(said(&m), "finished");
    }

    /// Events are keyed to THIS panel's request: another id, or no
    /// request at all, changes nothing on screen. A stale answer
    /// outliving a cancel must not repaint the panel it left.
    #[test]
    fn a_foreign_or_orphaned_event_changes_nothing() {
        let mut m = Photo::new();
        // No request out: every kind is a shrug.
        for ev in [
            Event::Hello { proto: 0, backends: vec!["local".into()] },
            Event::Delta { id: 1, text: "x".into() },
            Event::Done { id: 1, body: json!({}) },
            Event::Approval { id: 1, desc: "d".into() },
            Event::Progress { id: 1, msg: "m".into() },
            Event::Error { id: 1, msg: "e".into() },
        ] {
            assert!(!m.on_event(&ev), "{ev:?} with no request out");
        }
        assert_eq!(m.phase(), &Phase::Ready);

        // A request out, events about a different one.
        m.sent(7);
        assert!(!m.on_event(&Event::Error { id: 9, msg: "someone else's".into() }));
        assert!(!m.on_event(&Event::Progress { id: 9, msg: "theirs".into() }));
        assert_eq!(m.phase(), &Phase::Working { note: String::new() });
        assert_eq!(m.request(), Some(7));
    }

    /// The socket going away takes the request with it: the panel goes
    /// back to its list, because a "working" nothing is working on is
    /// a lie by spinner.
    #[test]
    fn a_lost_connection_ends_the_request() {
        let mut m = Photo::new();
        m.sent(5);
        m.on_event(&Event::Progress { id: 5, msg: "reading".into() });
        m.connection_lost();
        assert_eq!(m.phase(), &Phase::Ready);
        assert_eq!(m.request(), None);
        // And the request that died cannot speak from the grave.
        assert!(!m.on_event(&Event::Done { id: 5, body: json!({}) }));
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use nacelle::focus::Mods;

    fn ev(key: Key) -> KeyEv {
        KeyEv { key, mods: Mods::NONE, repeat: false, text: None }
    }

    fn typed(m: &mut Photo, s: &str) {
        for c in s.chars() {
            assert_eq!(m.key(&ev(Key::Char(c))), Outcome::Edited);
        }
    }

    #[test]
    fn typing_edits_the_path_and_the_arrows_walk_the_actions() {
        let mut m = Photo::new();
        typed(&mut m, "~/pix/a.png");
        assert_eq!(m.path(), "~/pix/a.png");
        assert_eq!(m.cursor(), 0);

        assert_eq!(m.key(&ev(Key::Down)), Outcome::Moved);
        assert_eq!(m.cursor(), 1);
        assert_eq!(m.key(&ev(Key::Up)), Outcome::Moved);
        assert_eq!(m.cursor(), 0);
        // The ends hold, and the key is NOT consumed there: an arrow
        // that moved nothing is an arrow to pass on.
        assert_eq!(m.key(&ev(Key::Up)), Outcome::Ignored);
        for _ in 0..10 {
            m.key(&ev(Key::Down));
        }
        assert_eq!(m.cursor(), ACTIONS.len() - 1);
        assert_eq!(m.key(&ev(Key::Down)), Outcome::Ignored);
    }

    #[test]
    fn enter_runs_the_chosen_action_on_the_typed_path() {
        let mut m = Photo::new();
        typed(&mut m, "/p/a.png");
        m.key(&ev(Key::Down));
        assert_eq!(m.key(&ev(Key::Enter)), Outcome::Run(1));
        // Run does not send — the caller does — so the phase moves on
        // `sent`, not before.
        assert_eq!(m.phase(), &Phase::Ready);
        m.sent(1);
        assert_eq!(m.phase(), &Phase::Working { note: String::new() });
    }

    /// An action over an empty box asks the daemon nothing: the panel
    /// answers for itself, in its own words, because this is the one
    /// case where there is no question to send.
    #[test]
    fn an_empty_path_is_answered_here_and_never_sent() {
        let mut m = Photo::new();
        assert_eq!(m.key(&ev(Key::Enter)), Outcome::Moved);
        assert_eq!(m.phase(), &Phase::Said { text: NEED_PATH.into() });
        assert_eq!(m.request(), None);
        // Whitespace is not a path either.
        let mut m = Photo::new();
        typed(&mut m, "   ");
        assert_eq!(m.key(&ev(Key::Enter)), Outcome::Moved);
        assert_eq!(m.phase(), &Phase::Said { text: NEED_PATH.into() });
    }

    #[test]
    fn a_click_runs_by_row_and_an_impossible_row_runs_nothing() {
        let mut m = Photo::new();
        typed(&mut m, "/p/a.png");
        assert_eq!(m.run(2), Outcome::Run(2));
        assert_eq!(m.cursor(), 2, "a click chooses the row it ran");
        // A hit key from a view this widget does not own resolves to
        // nothing rather than to row zero.
        assert_eq!(m.run(ACTIONS.len()), Outcome::Ignored);
    }

    #[test]
    fn escape_empties_the_path_and_then_belongs_to_the_caller() {
        let mut m = Photo::new();
        typed(&mut m, "/p");
        assert_eq!(m.key(&ev(Key::Escape)), Outcome::Edited);
        assert_eq!(m.path(), "");
        assert_eq!(m.key(&ev(Key::Escape)), Outcome::Ignored);
    }

    /// While the daemon holds the request, Escape means cancel and
    /// nothing else is typed blind under the message.
    #[test]
    fn a_running_request_takes_escape_as_cancel_and_nothing_else() {
        let mut m = Photo::new();
        typed(&mut m, "/p/a.png");
        m.sent(7);
        assert_eq!(m.key(&ev(Key::Char('x'))), Outcome::Ignored);
        assert_eq!(m.path(), "/p/a.png", "and the path did not eat it");
        assert_eq!(m.key(&ev(Key::Down)), Outcome::Ignored);
        assert_eq!(m.key(&ev(Key::Escape)), Outcome::Cancel(7));
        assert_eq!(m.phase(), &Phase::Ready);
        assert_eq!(m.request(), None);
    }

    /// The approval line is the same story: this panel cannot approve
    /// yet, so the only word it has is no.
    #[test]
    fn waiting_at_the_approval_line_cancels_the_same_way() {
        let mut m = Photo::new();
        typed(&mut m, "/p/a.png");
        m.sent(3);
        m.on_event(&Event::Approval { id: 3, desc: "run ffmpeg".into() });
        assert_eq!(m.key(&ev(Key::Char('y'))), Outcome::Ignored, "no key is a yes");
        assert_eq!(m.key(&ev(Key::Escape)), Outcome::Cancel(3));
    }

    /// A final message steps aside the moment the user moves on:
    /// editing the path puts the list back, Enter acknowledges, and
    /// nothing is silently re-run.
    #[test]
    fn a_final_message_yields_to_the_next_edit() {
        let mut m = Photo::new();
        typed(&mut m, "/p/a.png");
        m.sent(1);
        m.on_event(&Event::Error { id: 1, msg: "not built yet".into() });
        assert!(matches!(m.phase(), Phase::Said { .. }));

        assert_eq!(m.key(&ev(Key::Char('x'))), Outcome::Edited);
        assert_eq!(m.phase(), &Phase::Ready);
        assert_eq!(m.path(), "/p/a.pngx");

        // Enter under a message: read, dismissed, not re-run.
        m.sent(2);
        m.on_event(&Event::Error { id: 2, msg: "not built yet".into() });
        assert_eq!(m.key(&ev(Key::Enter)), Outcome::Moved);
        assert_eq!(m.phase(), &Phase::Ready);

        // Escape under a message: the same, with no cancel — there is
        // no request left to cancel.
        m.sent(3);
        m.on_event(&Event::Error { id: 3, msg: "not built yet".into() });
        assert_eq!(m.key(&ev(Key::Escape)), Outcome::Moved);
        assert_eq!(m.phase(), &Phase::Ready);
    }

    #[test]
    fn the_path_is_bounded_and_a_refused_edit_changes_nothing() {
        let mut m = Photo::new();
        m.input.set_value(&"x".repeat(MAX_PATH));
        assert_eq!(m.key(&ev(Key::Char('y'))), Outcome::Ignored);
        assert_eq!(m.path().chars().count(), MAX_PATH);
    }
}
