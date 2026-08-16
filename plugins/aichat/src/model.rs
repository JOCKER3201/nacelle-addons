//! What the CHAT panel KNOWS: the prompt field, the transcript, the one
//! request in flight, the question waiting at the confidentiality line,
//! and which backend the next question goes to.
//!
//! No drawing, no theme, no socket — the daemon's events are HANDED in
//! ([`Chat::on_event`]) and every command the panel sends leaves from
//! the widget half, after this model has answered what a user action
//! MEANT. That split is what makes the whole of "what does this event
//! do to the screen" testable without a window, and it is also the
//! decision file's own line: every command is caused by a user action,
//! so the model never talks — it only says what to say.
//!
//! # The field is not written here
//!
//! The prompt is a [`InputModel`] from the toolkit, driven through the
//! toolkit's own [`key_msg`] — caret motion, undo groups, selection and
//! the IME contract are decided in one place for the whole project,
//! exactly as the search panel's `finder` does it.

use nacelle::focus::KeyEv;
use nacelle::object::text_input::{key_msg, InputEdited, InputModel};
use nacelle_ai_client::{Backend, Event};

/// The longest prompt the box will hold, in characters — the search
/// panel's own cap, for the search panel's own reason: the field view
/// records a caret position per character every frame, each measured
/// from the start of the value, so the drawing cost of a prompt grows
/// with its SQUARE. Unbounded, one pathological paste would be a frozen
/// desktop.
pub const MAX_PROMPT: usize = 256;

/// The most turns the transcript keeps. Behaviour, not look: past this
/// the OLDEST turn leaves, because a chat panel is a conversation and
/// not an archive, and an unbounded transcript is a wrap of unbounded
/// cost on every frame the content moves.
pub const MAX_TURNS: usize = 200;

/// Who a transcript turn belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    /// The user's own words, as sent.
    You,
    /// The daemon's answer, grown a delta at a time.
    Ai,
    /// The panel talking about the conversation — an error the daemon
    /// reported, a cancel — never content anybody wrote.
    Note,
}

/// One turn of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub who: Who,
    pub text: String,
}

/// The panel's own words for a daemon that hung up mid-answer. A note,
/// because a question left streaming forever would read as an answer
/// that is still coming.
pub const SAY_GONE: &str = "the daemon went away; the answer is lost";

/// What one key meant. The caller redraws on anything but [`Ignored`],
/// and [`Ignored`] is also the answer that says "this key was not mine".
///
/// [`Ignored`]: Outcome::Ignored
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Nothing here answers this key.
    Ignored,
    /// The prompt text changed.
    Edited,
    /// The caret or the selection moved; the text did not.
    Moved,
    /// Enter over a settled, non-empty prompt with nothing in flight:
    /// the caller takes the prompt and asks the daemon.
    Submit,
    /// Escape over an empty box while a request runs: the caller sends
    /// `cancel` for it.
    CancelPending,
}

/// The panel's model.
pub struct Chat {
    /// The prompt field's state — the toolkit's, not this crate's.
    pub input: InputModel,
    turns: Vec<Turn>,
    /// The request in flight, by the id the client allocated for it.
    /// One at a time, deliberately: a chat is a turn-taking, and a
    /// second question streaming into the first one's answer is a
    /// transcript nobody can read.
    pending: Option<u64>,
    /// Index into `turns` of the answer being streamed, once the first
    /// delta has arrived.
    answer: Option<usize>,
    /// The question waiting at the confidentiality line: the request it
    /// belongs to, and the daemon's description of what it wants to do.
    approval: Option<(u64, String)>,
    /// The daemon's latest progress sentence for the request in flight.
    progress: Option<String>,
    backend: Backend,
    /// Moves on every change a frame could draw differently — the
    /// layout cache's whole invalidation key, beside the theme epoch.
    stamp: u64,
}

impl Chat {
    pub fn new() -> Chat {
        Chat {
            input: InputModel::new().with_max_len(MAX_PROMPT),
            turns: Vec::new(),
            pending: None,
            answer: None,
            approval: None,
            progress: None,
            // The API backend, because the decision file's toggle reads
            // CLAUDE/LOCAL in that order. `auto` is the daemon's own
            // word and deliberately not offered here: a chat whose
            // reader cannot tell who will answer is the thing the
            // toggle exists to prevent.
            backend: Backend::Claude,
            stamp: 0,
        }
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// CLAUDE ⇄ LOCAL — the chrome toggle's whole cycle.
    pub fn cycle_backend(&mut self) {
        self.backend = match self.backend {
            Backend::Local => Backend::Claude,
            _ => Backend::Local,
        };
        self.stamp += 1;
    }

    pub fn pending(&self) -> Option<u64> {
        self.pending
    }

    pub fn approval(&self) -> Option<(u64, &str)> {
        self.approval.as_ref().map(|(id, d)| (*id, d.as_str()))
    }

    pub fn progress(&self) -> Option<&str> {
        self.progress.as_deref()
    }

    /// The layout cache's invalidation key: moves whenever the
    /// transcript, the progress line or the approval block changed.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// One turn onto the transcript, oldest turn out past the cap.
    fn push(&mut self, who: Who, text: String) -> usize {
        self.turns.push(Turn { who, text });
        if self.turns.len() > MAX_TURNS {
            self.turns.remove(0);
            // The streamed answer moved down a slot with everything
            // else; an index into a shifted list would append the next
            // delta to the wrong turn.
            if let Some(a) = self.answer.as_mut() {
                *a = a.saturating_sub(1);
            }
        }
        self.stamp += 1;
        self.turns.len() - 1
    }

    /// The prompt leaves the box and joins the transcript. The caller
    /// sends it — this model owns no socket.
    pub fn take_prompt(&mut self) -> String {
        let text = self.input.value().to_string();
        self.input.set_value("");
        self.push(Who::You, text.clone());
        text
    }

    /// The client took the question and allocated `id` for it.
    pub fn sent(&mut self, id: u64) {
        self.pending = Some(id);
        self.answer = None;
        self.stamp += 1;
    }

    /// The client answered `None`: the daemon went away between the
    /// frame that drew Connected and the Enter. Said in the transcript,
    /// because the prompt is already there and silence after it would
    /// read as an answer that never comes.
    pub fn not_sent(&mut self) {
        self.push(Who::Note, "the daemon is offline; nothing was sent".to_string());
    }

    /// The user cancelled the request in flight (the caller has already
    /// told the daemon).
    pub fn cancelled(&mut self) {
        self.pending = None;
        self.answer = None;
        self.progress = None;
        self.approval = None;
        self.push(Who::Note, "cancelled".to_string());
    }

    /// The socket died under the request in flight. The flight is over
    /// — no delta will ever finish that answer — and the transcript
    /// says so; a panel with nothing in flight lost nothing.
    pub fn connection_lost(&mut self) {
        if self.pending.is_none() {
            return;
        }
        self.pending = None;
        self.answer = None;
        self.progress = None;
        self.approval = None;
        self.push(Who::Note, SAY_GONE.to_string());
    }

    /// The user pressed ALLOW or DENY: the question comes off the
    /// screen, and the id it belonged to goes back to the caller, whose
    /// client sends the `approve` line. `None` when there was nothing
    /// to answer — a click that raced the daemon's own error.
    pub fn take_approval(&mut self) -> Option<u64> {
        let (id, _) = self.approval.take()?;
        self.stamp += 1;
        Some(id)
    }

    /// One key. What it does is decided here; what it CAUSES — ask,
    /// cancel — is the caller's, so that every command line can be
    /// traced to a user action.
    pub fn key(&mut self, ev: &KeyEv) -> Outcome {
        let Some(msg) = key_msg(ev) else { return Outcome::Ignored };
        match self.input.apply(msg) {
            InputEdited::Edited => {
                self.stamp += 1;
                Outcome::Edited
            }
            InputEdited::Moved => Outcome::Moved,
            InputEdited::Submit => {
                // Enter means ask — once the running question is done.
                // A prompt of pure whitespace is not a question, and
                // consuming the Enter for it would eat the key that
                // does nothing.
                if self.pending.is_some() || self.input.value().trim().is_empty() {
                    return Outcome::Ignored;
                }
                Outcome::Submit
            }
            InputEdited::Cancel => {
                // Escape empties the box first; over an empty box it
                // reaches for the request in flight; with neither it is
                // not this panel's key.
                if !self.input.value().is_empty() {
                    self.input.set_value("");
                    self.stamp += 1;
                    return Outcome::Edited;
                }
                if self.pending.is_some() {
                    return Outcome::CancelPending;
                }
                Outcome::Ignored
            }
            // The clipboard does not cross the plugin boundary — the
            // search panel's shrug, for the search panel's reason.
            InputEdited::CopyRequest { .. } | InputEdited::PasteRequest => Outcome::Ignored,
            InputEdited::Rejected | InputEdited::None => Outcome::Ignored,
        }
    }

    /// One event off the socket, mapped onto the screen's state.
    ///
    /// Everything here is keyed to the request THIS panel has in
    /// flight: the daemon serves four widgets over one protocol, and an
    /// id this panel never allocated is another panel's conversation —
    /// stepped over exactly as the parser steps over an event from the
    /// future. (Each widget owns its own client and socket, so this is
    /// belt and braces; the braces are what the tests hold.)
    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Delta { id, text } => {
                if Some(id) != self.pending {
                    return;
                }
                match self.answer {
                    Some(i) => {
                        if let Some(t) = self.turns.get_mut(i) {
                            t.text.push_str(&text);
                        }
                        self.stamp += 1;
                    }
                    None => {
                        let i = self.push(Who::Ai, text);
                        self.answer = Some(i);
                    }
                }
            }
            Event::Done { id, .. } => {
                if Some(id) != self.pending {
                    return;
                }
                self.pending = None;
                self.answer = None;
                self.progress = None;
                // A question the daemon closed is a question nobody can
                // answer any more.
                self.approval = None;
                self.stamp += 1;
            }
            Event::Approval { id, desc } => {
                if Some(id) != self.pending {
                    return;
                }
                self.approval = Some((id, desc));
                self.stamp += 1;
            }
            Event::Progress { id, msg } => {
                if Some(id) != self.pending {
                    return;
                }
                self.progress = Some(msg);
                self.stamp += 1;
            }
            Event::Error { id, msg } => {
                if Some(id) != self.pending {
                    return;
                }
                self.pending = None;
                self.answer = None;
                self.progress = None;
                self.approval = None;
                self.push(Who::Note, msg);
            }
            // The handshake names the daemon's backends. This panel's
            // toggle is the closed CLAUDE/LOCAL pair either way, so the
            // list changes nothing a frame would draw.
            Event::Hello { .. } => {}
        }
    }
}

impl Default for Chat {
    fn default() -> Self {
        Chat::new()
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nacelle::focus::{Key, Mods};
    use nacelle_ai_client::parse_event;

    fn ev(key: Key) -> KeyEv {
        KeyEv { key, mods: Mods::NONE, repeat: false, text: None }
    }

    fn typed(c: &mut Chat, s: &str) {
        for ch in s.chars() {
            assert_eq!(c.key(&ev(Key::Char(ch))), Outcome::Edited);
        }
    }

    /// An event as the daemon would actually send it: the literal
    /// protocol line, through the real parser — so these tests pin the
    /// WIRE to the screen, not one enum to another.
    fn wire(c: &mut Chat, line: &str) {
        c.on_event(parse_event(line).expect("a line the spec spells must parse"));
    }

    /// The whole happy path: a typed question, the id the client gave
    /// it, deltas growing one answer turn, done ending the flight.
    #[test]
    fn a_question_streams_into_one_answer_turn() {
        let mut c = Chat::new();
        typed(&mut c, "why");
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Submit);
        let text = c.take_prompt();
        assert_eq!(text, "why");
        assert_eq!(c.input.value(), "", "the prompt left the box");
        assert_eq!(c.turns(), &[Turn { who: Who::You, text: "why".into() }]);
        c.sent(7);
        assert_eq!(c.pending(), Some(7));

        wire(&mut c, r#"{"ev":"delta","id":7,"text":"because "}"#);
        wire(&mut c, r#"{"ev":"delta","id":7,"text":"it is"}"#);
        assert_eq!(c.turns().len(), 2, "many deltas, ONE answer turn");
        assert_eq!(c.turns()[1], Turn { who: Who::Ai, text: "because it is".into() });

        wire(&mut c, r#"{"ev":"done","id":7}"#);
        assert_eq!(c.pending(), None);
        assert_eq!(c.turns().len(), 2, "done adds nothing to the transcript");
    }

    /// Another panel's conversation must not print here: an id this
    /// panel never allocated changes nothing on its screen.
    #[test]
    fn a_foreign_id_is_someone_elses_conversation() {
        let mut c = Chat::new();
        typed(&mut c, "q");
        c.take_prompt();
        c.sent(1);
        let stamp = c.stamp();
        wire(&mut c, r#"{"ev":"delta","id":9,"text":"not yours"}"#);
        wire(&mut c, r#"{"ev":"done","id":9}"#);
        wire(&mut c, r#"{"ev":"approval","id":9,"desc":"run something"}"#);
        wire(&mut c, r#"{"ev":"progress","id":9,"msg":"working"}"#);
        wire(&mut c, r#"{"ev":"error","id":9,"msg":"broke"}"#);
        assert_eq!(c.turns().len(), 1, "only the question this panel asked");
        assert_eq!(c.pending(), Some(1), "and it is still in flight");
        assert_eq!(c.approval(), None);
        assert_eq!(c.progress(), None);
        assert_eq!(c.stamp(), stamp, "nothing to redraw for");
    }

    /// The confidentiality line: the daemon asks, the panel shows, the
    /// user answers, the id goes back for the `approve` command.
    #[test]
    fn an_approval_waits_for_the_user_and_hands_back_its_id() {
        let mut c = Chat::new();
        typed(&mut c, "sort my downloads");
        c.take_prompt();
        c.sent(3);
        wire(&mut c, r#"{"ev":"approval","id":3,"desc":"run ffmpeg on clip.mp4"}"#);
        assert_eq!(c.approval(), Some((3, "run ffmpeg on clip.mp4")));
        // ALLOW (or DENY — the model's half is the same): the question
        // comes down and the caller sends the answer for this id.
        assert_eq!(c.take_approval(), Some(3));
        assert_eq!(c.approval(), None);
        // A second click has nothing left to answer.
        assert_eq!(c.take_approval(), None);
        // The request itself is still in flight — approve is not done.
        assert_eq!(c.pending(), Some(3));
    }

    /// Progress is the latest sentence and only the latest; done takes
    /// it down with the flight.
    #[test]
    fn progress_is_the_latest_sentence_and_ends_with_the_request() {
        let mut c = Chat::new();
        typed(&mut c, "x");
        c.take_prompt();
        c.sent(2);
        wire(&mut c, r#"{"ev":"progress","id":2,"msg":"reading"}"#);
        wire(&mut c, r#"{"ev":"progress","id":2,"msg":"thinking"}"#);
        assert_eq!(c.progress(), Some("thinking"));
        wire(&mut c, r#"{"ev":"done","id":2}"#);
        assert_eq!(c.progress(), None);
    }

    /// An error ends the flight and SAYS SO in the transcript — as the
    /// panel's own note, never as words anybody wrote.
    #[test]
    fn an_error_ends_the_flight_and_is_said_as_a_note() {
        let mut c = Chat::new();
        typed(&mut c, "x");
        c.take_prompt();
        c.sent(4);
        wire(&mut c, r#"{"ev":"approval","id":4,"desc":"touch a file"}"#);
        wire(&mut c, r#"{"ev":"error","id":4,"msg":"no such backend"}"#);
        assert_eq!(c.pending(), None);
        assert_eq!(c.approval(), None, "a dead request's question dies with it");
        assert_eq!(c.turns().last(), Some(&Turn { who: Who::Note, text: "no such backend".into() }));
    }

    /// Enter is Submit exactly when there is a question to ask and room
    /// to ask it: not over whitespace, not while one is in flight.
    #[test]
    fn enter_submits_a_question_and_only_a_question() {
        let mut c = Chat::new();
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Ignored, "an empty box has nothing to ask");
        typed(&mut c, "   ");
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Ignored, "whitespace is not a question");
        c.input.set_value("real question");
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Submit);
        c.take_prompt();
        c.sent(1);
        c.input.set_value("second question");
        assert_eq!(
            c.key(&ev(Key::Enter)),
            Outcome::Ignored,
            "one at a time: a second ask must wait for the first answer"
        );
    }

    /// Escape: the box first, the flight second, nobody's third.
    #[test]
    fn escape_clears_the_box_then_reaches_for_the_flight() {
        let mut c = Chat::new();
        typed(&mut c, "half a thou");
        c.input.set_value("half a thought");
        assert_eq!(c.key(&ev(Key::Escape)), Outcome::Edited);
        assert_eq!(c.input.value(), "");
        assert_eq!(c.key(&ev(Key::Escape)), Outcome::Ignored, "nothing in flight either");
        c.input.set_value("q");
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Submit);
        c.take_prompt();
        c.sent(5);
        assert_eq!(c.key(&ev(Key::Escape)), Outcome::CancelPending);
        c.cancelled();
        assert_eq!(c.pending(), None);
        assert_eq!(c.turns().last().map(|t| t.who), Some(Who::Note));
    }

    /// The transcript is bounded, the oldest turn leaves first, and the
    /// answer being streamed survives the shift.
    #[test]
    fn the_transcript_is_bounded_and_streaming_survives_the_trim() {
        let mut c = Chat::new();
        for i in 0..MAX_TURNS {
            c.push(Who::You, format!("turn {i}"));
        }
        assert_eq!(c.turns().len(), MAX_TURNS);
        c.sent(1);
        wire(&mut c, r#"{"ev":"delta","id":1,"text":"the answer"}"#);
        assert_eq!(c.turns().len(), MAX_TURNS, "the cap held: one in, one out");
        assert_eq!(c.turns()[0].text, "turn 1", "the OLDEST turn left");
        // The next delta still lands on the answer, not on a neighbour.
        wire(&mut c, r#"{"ev":"delta","id":1,"text":" grows"}"#);
        assert_eq!(c.turns().last().unwrap().text, "the answer grows");
    }

    /// The chrome toggle: CLAUDE ⇄ LOCAL, never auto — a chat whose
    /// reader cannot tell who will answer defeats the toggle.
    #[test]
    fn the_backend_toggle_cycles_the_two_words_the_decision_names() {
        let mut c = Chat::new();
        assert_eq!(c.backend(), Backend::Claude);
        c.cycle_backend();
        assert_eq!(c.backend(), Backend::Local);
        c.cycle_backend();
        assert_eq!(c.backend(), Backend::Claude);
        assert_eq!(Backend::Claude.word(), "claude");
        assert_eq!(Backend::Local.word(), "local");
    }

    /// The daemon's hello changes nothing a frame would draw: the
    /// toggle is a closed pair whatever the daemon installed.
    #[test]
    fn hello_is_taken_and_changes_nothing() {
        let mut c = Chat::new();
        let stamp = c.stamp();
        wire(&mut c, r#"{"ev":"hello","proto":0,"backends":["claude","local"]}"#);
        assert_eq!(c.stamp(), stamp);
        assert_eq!(c.turns().len(), 0);
    }

    /// The daemon hanging up mid-answer is the failure the panel must
    /// not render as eternal streaming: the flight dies with the socket
    /// and the transcript says so. With nothing in flight, nothing is
    /// lost and nothing is said.
    #[test]
    fn a_lost_connection_ends_the_flight_and_says_so() {
        let mut c = Chat::new();
        typed(&mut c, "q");
        c.take_prompt();
        c.sent(6);
        wire(&mut c, r#"{"ev":"approval","id":6,"desc":"touch a file"}"#);
        wire(&mut c, r#"{"ev":"progress","id":6,"msg":"thinking"}"#);
        c.connection_lost();
        assert_eq!(c.pending(), None);
        assert_eq!(c.approval(), None, "a dead request's question dies with it");
        assert_eq!(c.progress(), None);
        assert_eq!(c.turns().last(), Some(&Turn { who: Who::Note, text: SAY_GONE.into() }));
        // Idle, the hang-up is not news: the transcript gains nothing.
        let mut idle = Chat::new();
        idle.connection_lost();
        assert_eq!(idle.turns().len(), 0);
    }

    /// The offline race: Enter on the frame the daemon died. The
    /// question is already on the transcript, so the panel says what
    /// happened to it rather than leaving it to read as unanswered.
    #[test]
    fn a_question_the_client_refused_is_said_not_swallowed() {
        let mut c = Chat::new();
        c.input.set_value("q");
        assert_eq!(c.key(&ev(Key::Enter)), Outcome::Submit);
        c.take_prompt();
        c.not_sent();
        assert_eq!(c.pending(), None);
        assert_eq!(c.turns().len(), 2);
        assert_eq!(c.turns()[1].who, Who::Note);
    }
}
