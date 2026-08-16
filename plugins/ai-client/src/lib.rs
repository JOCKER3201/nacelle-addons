//! The nacelle-ai daemon protocol, client half — v0, JSON Lines over
//! one Unix socket.
//!
//! This crate is the shared floor the AI widgets stand on, the way
//! `launcher-core` is the launcher's. It knows three things and nothing
//! else: where the daemon's socket lives, how a command is spelled on
//! the way out, and how an event is read on the way in. It draws
//! nothing, owns no theme token, and starts no thread — a widget calls
//! [`AiClient::poll`] once per frame from the draw path it already has,
//! and everything here is a bounded amount of non-blocking work inside
//! that call.
//!
//! # Why there is no thread
//!
//! Because the widgets this crate serves are polled by a compositor at
//! a frame rate, and a frame is already a clock. A reader thread would
//! buy nothing but a channel, a mutex and a shutdown order — three
//! things to get wrong for data that is only ever LOOKED AT during a
//! frame anyway. So the socket is non-blocking, the frame reads what
//! has arrived, and a slow daemon costs a later frame, never a stuck
//! one.
//!
//! # Why a frame counter and not a timer
//!
//! Reconnection backs off by COUNTING POLLS ([`RETRY_FRAMES`] of them
//! between attempts), not by asking a clock. The polling cadence is the
//! widget's own frame rate, so "how often to knock" degrades exactly
//! with how alive the surface is: an occluded panel that is not drawn
//! does not knock at all, which is the correct amount of knocking for a
//! panel nobody can see. A timer would keep knocking for it.
//!
//! # Offline is a state, not an error
//!
//! The daemon is started beside the desktop and may be gone — not yet
//! up, crashed, or deliberately not installed. A missing daemon
//! therefore surfaces as [`Status::Offline`], a value a widget renders
//! as its empty state, and never as an `Err` a widget has to invent a
//! policy for. The client keeps trying, spaced by the counter above,
//! for as long as it is polled.
//!
//! # Protocol v0, field by field
//!
//! One JSON object per line, `\n`-terminated, spelled in
//! `.gap-program/decyzja-nacelle-ai-daemon.md` (the binding copy):
//!
//! * commands, client → daemon: `hello`, `ask`, `tool`, `approve`,
//!   `cancel` — built by [`cmd`];
//! * events, daemon → client: `hello`, `delta`, `done`, `approval`,
//!   `progress`, `error` — parsed by [`parse_event`] into [`Event`].
//!
//! A line this version does not recognise is DROPPED, silently: v1 will
//! speak over the same socket, and a v0 widget meeting a v1 event
//! should miss it, not die of it.

use serde_json::Value;
use std::collections::VecDeque;
use std::env;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// The protocol generation this crate speaks, sent in `hello` and
/// expected back in the daemon's.
pub const PROTO: u64 = 0;

/// Polls between connection attempts while the daemon is away.
///
/// A count of frames, not a duration — see the head of this file for
/// why. At a common 60 Hz this knocks about every two seconds, which is
/// prompt enough that a daemon started by hand answers while the hand
/// is still on the keyboard, and idle enough that a machine without the
/// daemon is not being rung sixty times a second forever.
pub const RETRY_FRAMES: u32 = 120;

/// The longest event line kept. A line past this is a peer speaking
/// some other protocol (or a daemon gone wrong), and the honest answer
/// is to drop that line whole rather than grow without bound holding
/// its beginning.
const MAX_LINE: usize = 1 << 20;

/// One read's worth of socket. Small enough to live on the stack of a
/// frame, large enough that a burst of deltas drains in a few loops.
const CHUNK: usize = 4096;

// ------------------------------------------------------------------ words

/// Whether the daemon is there. The whole health of the connection in
/// one word, because that is all a widget can honestly render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No socket. The client keeps knocking, spaced by the counter;
    /// a widget shows its empty state and queues nothing.
    Offline,
    /// A socket is open and `hello` has been queued on it. Commands
    /// go out; events come in with [`AiClient::poll`].
    Connected,
}

/// Who answers an `ask` — the protocol's closed word set, kept as an
/// enum so a widget cannot misspell a backend into a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Claude,
    Local,
    Auto,
}

impl Backend {
    /// The word the wire carries.
    pub fn word(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Local => "local",
            Backend::Auto => "auto",
        }
    }
}

/// The daemon's tools, one word each — the closed set v0 declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Media looping: a video in, a seamless loop out; photos in, a
    /// one-minute looped clip out.
    Loop,
    /// Photo processing.
    Photo,
    /// File sorting.
    Sort,
}

impl Tool {
    /// The word the wire carries.
    pub fn word(self) -> &'static str {
        match self {
            Tool::Loop => "loop",
            Tool::Photo => "photo",
            Tool::Sort => "sort",
        }
    }
}

/// One line from the daemon, read into the fields v0 declares.
///
/// `Done` keeps its whole line as [`Value`]: the spec spells it
/// `{"ev":"done","id":N,...}` and the trailing `...` is the result —
/// a tool's output path, an answer's closing facts — whose shape only
/// the daemon's side of the fence pins down. Handing the object over
/// whole lets a widget read what it knows and step over what it does
/// not, which is the same forbearance the parser shows unknown events.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The daemon's half of the handshake.
    Hello { proto: u64, backends: Vec<String> },
    /// One streamed piece of an answer to request `id`.
    Delta { id: u64, text: String },
    /// Request `id` is finished; `body` is the full event object.
    Done { id: u64, body: Value },
    /// Request `id` is stopped at the confidentiality line and waits
    /// for [`AiClient::approve`]. `desc` says what it wants to do.
    Approval { id: u64, desc: String },
    /// A progress note for request `id` — a sentence, not a fraction.
    Progress { id: u64, msg: String },
    /// Request `id` failed, and this is the daemon saying why.
    Error { id: u64, msg: String },
}

// --------------------------------------------------------------- commands

/// The five command lines v0 knows, spelled exactly as the decision
/// file spells them — key order and all, so a byte-for-byte reading of
/// a trace against the spec finds them equal. Text lands on the wire
/// through the JSON escaper, never bare, so a quote or a newline typed
/// into a chat field is a character in a string and not a second line
/// of protocol. No function here returns the terminating `\n`; the
/// line ending is the TRANSPORT's framing, and [`AiClient`] adds it at
/// the socket.
pub mod cmd {
    use super::{Backend, Tool, Value, PROTO};

    /// A string as JSON spells it, quotes and escapes included.
    ///
    /// `serde_json` cannot fail on a `&str`; the fallback exists so
    /// that no path in this module can panic inside somebody's frame.
    fn quoted(s: &str) -> String {
        serde_json::to_string(s).unwrap_or_else(|_| String::from("\"\""))
    }

    /// `{"cmd":"hello","client":"<name>","proto":0}` — the client's
    /// half of the handshake, first line on every fresh socket.
    pub fn hello(client: &str) -> String {
        format!("{{\"cmd\":\"hello\",\"client\":{},\"proto\":{}}}", quoted(client), PROTO)
    }

    /// `{"cmd":"ask","id":N,"text":"...","backend":"..."}`
    pub fn ask(id: u64, text: &str, backend: Backend) -> String {
        format!(
            "{{\"cmd\":\"ask\",\"id\":{},\"text\":{},\"backend\":\"{}\"}}",
            id,
            quoted(text),
            backend.word()
        )
    }

    /// `{"cmd":"tool","id":N,"tool":"...","args":{...}}`
    pub fn tool(id: u64, tool: Tool, args: &Value) -> String {
        let args = serde_json::to_string(args).unwrap_or_else(|_| String::from("null"));
        format!("{{\"cmd\":\"tool\",\"id\":{},\"tool\":\"{}\",\"args\":{}}}", id, tool.word(), args)
    }

    /// `{"cmd":"approve","id":N,"allow":true|false}` — the user's
    /// answer to an [`super::Event::Approval`], carried back across
    /// the same confidentiality line the question came over.
    pub fn approve(id: u64, allow: bool) -> String {
        format!("{{\"cmd\":\"approve\",\"id\":{},\"allow\":{}}}", id, allow)
    }

    /// `{"cmd":"cancel","id":N}`
    pub fn cancel(id: u64) -> String {
        format!("{{\"cmd\":\"cancel\",\"id\":{}}}", id)
    }
}

// ----------------------------------------------------------------- events

/// The string a field holds, owned, or `None` when the field is not
/// there or is not a string.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}

/// The `id` every event but `hello` carries.
fn id_field(v: &Value) -> Option<u64> {
    v.get("id")?.as_u64()
}

/// One event line, read by the fields v0 declares.
///
/// `None` is the parser's whole error surface, on purpose: a line that
/// is not JSON, not an object, names an event this version does not
/// know, or is missing a field its event requires, is a line to step
/// over — the socket stays up and the next line gets its own chance.
/// A client that tore the connection down over one bad line would turn
/// a daemon's single hiccup into every panel going Offline at once.
pub fn parse_event(line: &str) -> Option<Event> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("ev")?.as_str()? {
        "hello" => Some(Event::Hello {
            proto: v.get("proto")?.as_u64()?,
            // Absent is read as none installed: which backends exist is
            // the daemon's fact to volunteer, not this parser's to
            // require.
            backends: v
                .get("backends")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
        }),
        "delta" => Some(Event::Delta { id: id_field(&v)?, text: str_field(&v, "text")? }),
        "done" => Some(Event::Done { id: id_field(&v)?, body: v }),
        "approval" => Some(Event::Approval { id: id_field(&v)?, desc: str_field(&v, "desc")? }),
        "progress" => Some(Event::Progress { id: id_field(&v)?, msg: str_field(&v, "msg")? }),
        "error" => Some(Event::Error { id: id_field(&v)?, msg: str_field(&v, "msg")? }),
        _ => None,
    }
}

/// Bytes in, whole lines out — the seam between a socket that hands
/// over arbitrary chunks and a parser that wants one line at a time.
///
/// Free-standing and fed plain byte slices so the framing can be
/// exercised without a socket to read from: every split a peer could
/// deal — a line across two reads, three lines in one, a newline on a
/// chunk edge — is a `feed` call a test can write.
#[derive(Default)]
struct LineBuf {
    /// The current line so far, newline not yet seen.
    buf: Vec<u8>,
    /// True while discarding an overlong line: everything up to the
    /// next newline belongs to the line already given up on.
    skipping: bool,
}

impl LineBuf {
    /// Take one read's worth of bytes; push every completed line's
    /// event onto `events`.
    fn feed(&mut self, mut bytes: &[u8], events: &mut VecDeque<Event>) {
        while let Some(pos) = bytes.iter().position(|&b| b == b'\n') {
            let (head, rest) = bytes.split_at(pos);
            bytes = &rest[1..];
            if self.skipping {
                // The newline ends the line being discarded; the line
                // AFTER it is innocent.
                self.skipping = false;
                self.buf.clear();
                continue;
            }
            self.buf.extend_from_slice(head);
            // Non-UTF-8 is not this protocol; the line is dropped the
            // same way an unknown event is.
            if let Ok(line) = std::str::from_utf8(&self.buf) {
                if let Some(ev) = parse_event(line) {
                    events.push_back(ev);
                }
            }
            self.buf.clear();
        }
        if self.skipping {
            return;
        }
        self.buf.extend_from_slice(bytes);
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
            self.skipping = true;
        }
    }
}

// ----------------------------------------------------------------- socket

/// Where the daemon listens: `$XDG_RUNTIME_DIR/nacelle/ai.sock`, or
/// `/tmp/nacelle-$UID/ai.sock` on a system that sets no runtime dir.
/// The fallback keeps the UID in the path for the same reason the
/// runtime dir has it in its own: two users on one machine get two
/// sockets, not a fight over one.
pub fn socket_path() -> PathBuf {
    match env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => Path::new(&dir).join("nacelle").join("ai.sock"),
        _ => {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/tmp/nacelle-{uid}/ai.sock"))
        }
    }
}

/// The client: one non-blocking socket, one outbound buffer, one queue
/// of parsed events, driven entirely by [`AiClient::poll`].
///
/// A widget owns one of these, calls `poll` once per frame, drains
/// [`AiClient::take_event`] until `None`, and renders
/// [`AiClient::status`]. That is the whole contract; nothing here
/// blocks, spawns or sleeps.
pub struct AiClient {
    /// The name `hello` announces — the widget's own, so a daemon-side
    /// trace says which panel asked.
    client: String,
    path: PathBuf,
    stream: Option<UnixStream>,
    line: LineBuf,
    events: VecDeque<Event>,
    /// Commands not yet fully written. Almost always drains in the
    /// same poll it was filled; it exists so a full socket buffer is a
    /// short queue here rather than a lost or half-written line there.
    pending: Vec<u8>,
    /// Polls left before the next connection attempt. Meaningful only
    /// while Offline; zero means "knock on the next poll".
    until_retry: u32,
    /// The interval the counter above reloads from.
    retry_frames: u32,
    /// The next request id. Ids tell answers apart, so they only ever
    /// move forward; nothing reads meaning into the numbers.
    next_id: u64,
}

impl AiClient {
    /// A client for the daemon's own socket path, Offline until the
    /// first `poll` finds the daemon.
    pub fn new(client: &str) -> Self {
        Self::at(client, socket_path())
    }

    /// The same client aimed at an explicit path. This is the seam the
    /// tests connect through; a widget has no reason to pass anything
    /// but [`socket_path`]'s answer.
    pub fn at(client: &str, path: PathBuf) -> Self {
        AiClient {
            client: client.to_owned(),
            path,
            stream: None,
            line: LineBuf::default(),
            events: VecDeque::new(),
            pending: Vec::new(),
            // Zero, so the very first poll knocks: a daemon started
            // before the desktop is Connected on frame one instead of
            // an interval later.
            until_retry: 0,
            retry_frames: RETRY_FRAMES,
            next_id: 1,
        }
    }

    /// The same client with a different knock interval, for a caller
    /// whose frame rate makes the default wrong.
    pub fn with_retry_frames(mut self, frames: u32) -> Self {
        self.retry_frames = frames;
        self
    }

    /// Whether the daemon is there, as of the last `poll`.
    pub fn status(&self) -> Status {
        if self.stream.is_some() {
            Status::Connected
        } else {
            Status::Offline
        }
    }

    /// One frame's worth of I/O: reconnect if due, flush what waits,
    /// read what arrived. Non-blocking throughout — the frame gets the
    /// bytes the kernel already has and not one syscall's wait more.
    pub fn poll(&mut self) {
        if self.stream.is_none() {
            if self.until_retry > 0 {
                self.until_retry -= 1;
                return;
            }
            self.until_retry = self.retry_frames;
            self.connect();
            if self.stream.is_none() {
                return;
            }
        }
        self.flush();
        self.read();
    }

    /// The oldest event not yet taken, or `None` when the frame has
    /// read them all. Drained in a `while let` after `poll`.
    pub fn take_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Queue an `ask`; the allocated request id, or `None` while
    /// Offline. `None` means the command went NOWHERE — the caller
    /// shows the offline state, and nothing is silently spooled for a
    /// daemon that may be away all day.
    pub fn ask(&mut self, text: &str, backend: Backend) -> Option<u64> {
        if self.stream.is_none() {
            return None;
        }
        let id = self.take_id();
        self.send(&cmd::ask(id, text, backend));
        Some(id)
    }

    /// Queue a `tool` call; the allocated request id, or `None` while
    /// Offline, on the same reasoning as [`AiClient::ask`].
    pub fn tool(&mut self, tool: Tool, args: &Value) -> Option<u64> {
        if self.stream.is_none() {
            return None;
        }
        let id = self.take_id();
        self.send(&cmd::tool(id, tool, args));
        Some(id)
    }

    /// Answer an approval request. `false` when Offline — though a
    /// daemon that died between asking and being answered has dropped
    /// the question with the connection anyway.
    pub fn approve(&mut self, id: u64, allow: bool) -> bool {
        if self.stream.is_none() {
            return false;
        }
        self.send(&cmd::approve(id, allow));
        true
    }

    /// Cancel request `id`. `false` when Offline.
    pub fn cancel(&mut self, id: u64) -> bool {
        if self.stream.is_none() {
            return false;
        }
        self.send(&cmd::cancel(id));
        true
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// One connection attempt, silent either way. Failure is not
    /// reported because it is not news: Offline is already the state,
    /// and the counter already says when the next knock is.
    fn connect(&mut self) {
        let Ok(stream) = UnixStream::connect(&self.path) else { return };
        if stream.set_nonblocking(true).is_err() {
            // A socket that cannot be made non-blocking is a socket
            // that could stall a frame; better no connection at all.
            return;
        }
        self.stream = Some(stream);
        self.line = LineBuf::default();
        self.pending.clear();
        let hello = cmd::hello(&self.client);
        self.send(&hello);
    }

    /// Append one command line to the outbound buffer and push it at
    /// the socket. Terminating newline added here: framing belongs to
    /// the transport, not to the command spellings.
    fn send(&mut self, line: &str) {
        self.pending.extend_from_slice(line.as_bytes());
        self.pending.push(b'\n');
        self.flush();
    }

    fn flush(&mut self) {
        while !self.pending.is_empty() {
            let wrote = match self.stream.as_mut() {
                Some(s) => s.write(&self.pending),
                None => return,
            };
            match wrote {
                // A zero-byte write on a stream socket says the peer
                // is gone the same way an error does.
                Ok(0) => return self.drop_stream(),
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return self.drop_stream(),
            }
        }
    }

    fn read(&mut self) {
        let mut chunk = [0u8; CHUNK];
        loop {
            let got = match self.stream.as_mut() {
                Some(s) => s.read(&mut chunk),
                None => return,
            };
            match got {
                // EOF: the daemon hung up. Back to Offline, knock
                // again an interval from now.
                Ok(0) => return self.drop_stream(),
                Ok(n) => self.line.feed(&chunk[..n], &mut self.events),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return self.drop_stream(),
            }
        }
    }

    /// Back to Offline. Parsed events are KEPT — they arrived whole
    /// and are true regardless of what the socket did afterwards — but
    /// the half-line and the unsent commands go with the connection:
    /// a fresh socket starts with a fresh `hello`, not with the tail
    /// of the old conversation.
    fn drop_stream(&mut self) {
        self.stream = None;
        self.pending.clear();
        self.line = LineBuf::default();
        self.until_retry = self.retry_frames;
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod command_tests {
    use super::*;

    /// The five commands, spelled against the decision file's own
    /// lines. Byte-for-byte, key order included: the spec is binding
    /// field by field, and a trace laid beside it should read equal.
    #[test]
    fn every_command_is_spelled_as_the_spec_spells_it() {
        assert_eq!(
            cmd::hello("aichat"),
            r#"{"cmd":"hello","client":"aichat","proto":0}"#
        );
        assert_eq!(
            cmd::ask(7, "sort my downloads", Backend::Claude),
            r#"{"cmd":"ask","id":7,"text":"sort my downloads","backend":"claude"}"#
        );
        let args = serde_json::json!({ "path": "/tmp/clip.mp4" });
        assert_eq!(
            cmd::tool(3, Tool::Loop, &args),
            r#"{"cmd":"tool","id":3,"tool":"loop","args":{"path":"/tmp/clip.mp4"}}"#
        );
        assert_eq!(cmd::approve(4, true), r#"{"cmd":"approve","id":4,"allow":true}"#);
        assert_eq!(cmd::approve(4, false), r#"{"cmd":"approve","id":4,"allow":false}"#);
        assert_eq!(cmd::cancel(5), r#"{"cmd":"cancel","id":5}"#);
    }

    /// What a user types goes to the daemon as a STRING, whatever it
    /// holds. A quote, a backslash or a newline in a chat field must
    /// come out escaped — an unescaped newline would end the frame in
    /// the middle of the command and feed the daemon half a line plus
    /// garbage.
    #[test]
    fn typed_text_cannot_break_the_framing() {
        let hostile = "a \"quote\", a \\ and a\nnewline";
        let line = cmd::ask(1, hostile, Backend::Auto);
        assert!(!line.contains('\n'), "the frame's own newline must be the only one");
        // And the round trip gives the text back exactly.
        let v: Value = serde_json::from_str(&line).expect("a command must be valid JSON");
        assert_eq!(v["text"].as_str(), Some(hostile));
        assert_eq!(v["backend"].as_str(), Some("auto"));
        assert_eq!(v["id"].as_u64(), Some(1));
    }

    /// The wire words are the protocol's, pinned so a rename in the
    /// enum cannot silently rename them on the socket.
    #[test]
    fn the_wire_words_are_the_protocols_own() {
        assert_eq!(Backend::Claude.word(), "claude");
        assert_eq!(Backend::Local.word(), "local");
        assert_eq!(Backend::Auto.word(), "auto");
        assert_eq!(Tool::Loop.word(), "loop");
        assert_eq!(Tool::Photo.word(), "photo");
        assert_eq!(Tool::Sort.word(), "sort");
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn every_event_kind_is_read_by_its_fields() {
        assert_eq!(
            parse_event(r#"{"ev":"hello","proto":0,"backends":["claude","local"]}"#),
            Some(Event::Hello {
                proto: 0,
                backends: vec!["claude".into(), "local".into()]
            })
        );
        assert_eq!(
            parse_event(r#"{"ev":"delta","id":7,"text":"the answer so far"}"#),
            Some(Event::Delta { id: 7, text: "the answer so far".into() })
        );
        assert_eq!(
            parse_event(r#"{"ev":"approval","id":2,"desc":"run ffmpeg on clip.mp4"}"#),
            Some(Event::Approval { id: 2, desc: "run ffmpeg on clip.mp4".into() })
        );
        assert_eq!(
            parse_event(r#"{"ev":"progress","id":3,"msg":"encoding"}"#),
            Some(Event::Progress { id: 3, msg: "encoding".into() })
        );
        assert_eq!(
            parse_event(r#"{"ev":"error","id":4,"msg":"no such file"}"#),
            Some(Event::Error { id: 4, msg: "no such file".into() })
        );
    }

    /// `done` is spelled `{"ev":"done","id":N,...}` — the tail is the
    /// daemon's result, so the event hands the object over whole and a
    /// widget reads the fields it knows.
    #[test]
    fn done_carries_its_whole_line() {
        let ev = parse_event(r#"{"ev":"done","id":9,"out":"/tmp/clip-loop.mp4"}"#)
            .expect("a done with an id must parse");
        match ev {
            Event::Done { id, body } => {
                assert_eq!(id, 9);
                assert_eq!(body["out"].as_str(), Some("/tmp/clip-loop.mp4"));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A daemon that lists no backends still said hello; the list is
    /// its fact to volunteer, not the parser's to demand.
    #[test]
    fn a_hello_without_backends_is_a_hello_with_none() {
        assert_eq!(
            parse_event(r#"{"ev":"hello","proto":0}"#),
            Some(Event::Hello { proto: 0, backends: vec![] })
        );
    }

    /// The parser's whole refusal surface is `None`: not JSON, not an
    /// object, an unknown event, a missing required field. Each is a
    /// line to step over, never a reason to panic or disconnect.
    #[test]
    fn a_line_this_version_cannot_read_is_dropped_not_died_of() {
        for line in [
            "",                                     // nothing
            "not json at all",                      // not JSON
            "[1,2,3]",                              // JSON, not an object
            "42",                                   // JSON, a number
            r#"{"cmd":"ask","id":1}"#,              // a COMMAND, echoed back
            r#"{"ev":"v1-novelty","id":1}"#,        // an event from the future
            r#"{"ev":"delta","id":1}"#,             // delta without its text
            r#"{"ev":"delta","text":"x"}"#,         // delta without its id
            r#"{"ev":"approval","id":1}"#,          // approval without its desc
            r#"{"ev":"error","msg":"x"}"#,          // error without its id
            r#"{"ev":"hello"}"#,                    // hello without its proto
            r#"{"ev":"delta","id":-1,"text":"x"}"#, // an id no request was given
        ] {
            assert_eq!(parse_event(line), None, "line {line:?} must be dropped");
        }
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;

    /// Feed bytes through the line buffer, take the events out.
    fn fed(chunks: &[&[u8]]) -> Vec<Event> {
        let mut lines = LineBuf::default();
        let mut events = VecDeque::new();
        for chunk in chunks {
            lines.feed(chunk, &mut events);
        }
        events.into_iter().collect()
    }

    /// A socket owes nobody whole lines: one event may arrive over
    /// three reads, three events in one. Every split of the same bytes
    /// must read the same.
    #[test]
    fn events_are_read_whole_however_the_reads_split_them() {
        let wire = b"{\"ev\":\"delta\",\"id\":1,\"text\":\"a\"}\n{\"ev\":\"delta\",\"id\":1,\"text\":\"b\"}\n{\"ev\":\"done\",\"id\":1}\n";
        let all_at_once = fed(&[wire]);
        assert_eq!(all_at_once.len(), 3);
        assert_eq!(
            all_at_once[1],
            Event::Delta { id: 1, text: "b".into() }
        );
        assert!(matches!(all_at_once[2], Event::Done { id: 1, .. }));
        // Byte by byte — every boundary a read could land on.
        let dribbled = fed(&wire.iter().map(std::slice::from_ref).collect::<Vec<_>>());
        assert_eq!(dribbled, all_at_once);
        // And split right at a newline, the classic edge.
        let split = fed(&[&wire[..33], &wire[33..]]);
        assert_eq!(split, all_at_once);
    }

    /// Half a line is not an event yet. No newline, no parse — the
    /// fragment waits in the buffer for the read that completes it.
    #[test]
    fn a_line_is_nothing_until_its_newline_arrives() {
        let mut lines = LineBuf::default();
        let mut events = VecDeque::new();
        lines.feed(br#"{"ev":"delta","id":1,"text":"waiting"}"#, &mut events);
        assert!(events.is_empty(), "no newline has arrived, so no event has");
        lines.feed(b"\n", &mut events);
        assert_eq!(
            events.pop_front(),
            Some(Event::Delta { id: 1, text: "waiting".into() })
        );
    }

    /// One bad line costs that line and nothing after it: garbage,
    /// then a good event, reads as the good event.
    #[test]
    fn a_bad_line_does_not_take_its_neighbours_with_it() {
        let events = fed(&[b"utter garbage\n\xff\xfe\n{\"ev\":\"progress\",\"id\":2,\"msg\":\"ok\"}\n"]);
        assert_eq!(events, vec![Event::Progress { id: 2, msg: "ok".into() }]);
    }

    /// A line past [`MAX_LINE`] is dropped WHOLE — head already
    /// buffered and tail still coming — and the line after the next
    /// newline is read as if nothing happened. The buffer must not
    /// grow with the line it gave up on.
    #[test]
    fn an_overlong_line_is_dropped_whole_and_the_next_one_survives() {
        let mut lines = LineBuf::default();
        let mut events = VecDeque::new();
        let flood = vec![b'x'; MAX_LINE + 1];
        lines.feed(&flood, &mut events);
        assert!(lines.buf.is_empty(), "an abandoned line must not be kept");
        assert!(lines.skipping);
        // More of the same flood: still discarded, still not stored.
        lines.feed(&flood, &mut events);
        assert!(lines.buf.is_empty());
        // Its newline finally arrives, then an honest event.
        lines.feed(b"\n{\"ev\":\"error\",\"id\":8,\"msg\":\"late\"}\n", &mut events);
        let events: Vec<Event> = events.into_iter().collect();
        assert_eq!(events, vec![Event::Error { id: 8, msg: "late".into() }]);
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;

    /// A path no daemon listens on: the scratch dir does not even
    /// exist, so connect fails without touching anything real.
    fn nowhere() -> PathBuf {
        PathBuf::from("/nonexistent-nacelle-test/ai.sock")
    }

    /// The knock is counted in polls. One failed attempt arms the
    /// counter; the next attempt happens [`RETRY_FRAMES`] polls later
    /// and not one poll sooner. (The counter is observed directly —
    /// the seam exists so this logic is testable without a socket.)
    #[test]
    fn reconnection_is_paced_by_polls_not_by_a_clock() {
        let mut c = AiClient::at("test", nowhere()).with_retry_frames(3);
        assert_eq!(c.status(), Status::Offline);
        assert_eq!(c.until_retry, 0, "the first poll must knock at once");
        c.poll(); // knocks, fails, arms the counter
        assert_eq!(c.status(), Status::Offline);
        assert_eq!(c.until_retry, 3);
        c.poll();
        c.poll();
        c.poll();
        assert_eq!(c.until_retry, 0, "three waiting polls must spend the counter");
        c.poll(); // knocks again, fails again, rearms
        assert_eq!(c.until_retry, 3);
    }

    /// Offline is a state a widget renders, not an error it handles:
    /// requests answer `None`/`false` and queue NOTHING, so no command
    /// is spooled up for a daemon that may be away all day.
    #[test]
    fn offline_takes_no_commands_and_keeps_no_queue() {
        let mut c = AiClient::at("test", nowhere());
        c.poll();
        assert_eq!(c.ask("hello?", Backend::Auto), None);
        assert_eq!(c.tool(Tool::Sort, &serde_json::json!({})), None);
        assert!(!c.approve(1, true));
        assert!(!c.cancel(1));
        assert!(c.pending.is_empty(), "nothing may wait for a daemon that is not there");
        assert_eq!(c.take_event(), None);
    }

    /// Ids only move forward. Two widgets never share a client, but
    /// one widget's second request must not answer its first.
    #[test]
    fn request_ids_are_never_reused() {
        let mut c = AiClient::at("test", nowhere());
        let a = c.take_id();
        let b = c.take_id();
        assert!(b > a);
    }

    /// Polling while Offline is the steady state of a machine without
    /// the daemon, so it has to stay cheap and quiet forever — no
    /// growth, no events, no panic, however long it goes on.
    #[test]
    fn a_daemonless_machine_can_be_polled_forever() {
        let mut c = AiClient::at("test", nowhere()).with_retry_frames(2);
        for _ in 0..100 {
            c.poll();
            assert_eq!(c.status(), Status::Offline);
            assert_eq!(c.take_event(), None);
        }
        assert!(c.pending.is_empty());
        assert!(c.line.buf.is_empty());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// The two spellings the spec gives, pinned. The environment is
    /// NOT mutated here — tests share a process — so the fallback is
    /// exercised through its own arithmetic: both answers are what
    /// `socket_path` computes from each of the two worlds.
    #[test]
    fn the_socket_lives_where_the_spec_says() {
        // The runtime-dir world.
        let with_xdg = Path::new("/run/user/1000").join("nacelle").join("ai.sock");
        assert_eq!(with_xdg, Path::new("/run/user/1000/nacelle/ai.sock"));
        // The fallback world: per-UID under /tmp, never a shared file.
        let uid = unsafe { libc::getuid() };
        let fallback = PathBuf::from(format!("/tmp/nacelle-{uid}/ai.sock"));
        assert!(fallback.to_string_lossy().contains(&format!("nacelle-{uid}")));
        // And the path the running process would actually use is one
        // of the two shapes, whichever world this test runs in.
        let live = socket_path();
        let s = live.to_string_lossy();
        assert!(
            s.ends_with("/nacelle/ai.sock") || s.ends_with(&format!("/tmp/nacelle-{uid}/ai.sock")),
            "socket_path answered {s:?}, which is neither spelling the spec gives"
        );
    }
}
