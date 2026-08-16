//! CHAT panel — the conversation with the nacelle-ai daemon, on the
//! SEARCH AND AI board.
//!
//! A question typed into the prompt box goes to the daemon as one
//! `{"cmd":"ask"}` line; the answer streams back a `delta` at a time
//! into the transcript, live, until `done` closes the turn. Which of
//! the two backends the NEXT question goes to — CLAUDE or LOCAL — is
//! the title band's business: the band's right half names it, and a
//! click on the band cycles it. Both roads meet in the daemon; the
//! toggle only says who answers.
//!
//! # What the panel is made of
//!
//! Three things, top to bottom, inside the content box the host's panel
//! container hands over:
//!
//! 1. the TRANSCRIPT — the turns so far as a scrolling column, driven
//!    by the toolkit's own `view::scroll`: its physics, its snap, its
//!    bar, exactly as the file browser drives them. The view follows a
//!    streaming answer while it is at the bottom and stays put the
//!    moment the reader scrolls away — a transcript that yanks the view
//!    out from under a reader is not being read;
//! 2. the QUESTION ON THE TABLE — while the daemon stops at the
//!    confidentiality line, its `approval` event is a row above the
//!    prompt: the daemon's description, and ALLOW / DENY. The click's
//!    answer goes back as `{"cmd":"approve"}` under the same id. While
//!    a request merely runs, the row shows the daemon's latest
//!    `progress` sentence instead;
//! 3. the PROMPT BOX — the toolkit's text input, drawn by [`field`]
//!    exactly as the search box is. Enter asks; Escape empties the box,
//!    then cancels the request in flight.
//!
//! Every command line leaves this panel because a user did something —
//! an Enter, a click on ALLOW, an Escape. The panel never speaks on its
//! own: the daemon decision file's rule, kept at this end of the socket
//! too.
//!
//! # Offline is the empty state
//!
//! No daemon, no controls: the panel draws the theme's `[emptystate]`
//! message and nothing else, exactly as the old AI panel did — a field
//! that accepts a question nobody will ever read is the dead text field
//! that panel existed to avoid. The client keeps knocking (a counted
//! poll, not a timer) and the controls return with the daemon.
//!
//! Every colour, length and word comes from the theme. Nothing here
//! knows what a colour is: a missing token degrades through the raw
//! answers the ABI itself gives, never through a number that used to be
//! the design.

pub mod field;
pub mod model;

use crate::field::FieldView;
use crate::model::{Chat, Outcome, Turn, Who};
use nacelle::focus::{Key, KeyEv, Mods};
use nacelle::runtime::{
    keys, ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_NONE,
    SIZING_REFERENCE,
};
use nacelle::theme::parse::State;
use nacelle::theme::Color;
use nacelle::ui::Align;
use nacelle::view::paint::{self, RoleLook};
use nacelle::view::scroll::{
    self, scrollbar, Easing, ScrollPhysics, ScrollView, ScrollbarLook, Snap,
};
use nacelle::view::surface::{AbiSurface, Surface};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use nacelle_ai_client::{AiClient, Backend, Status};
use std::ffi::c_void;

/// The name the host's title band shows, and the name the client's
/// `hello` announces — one word, so a daemon-side trace and a screen
/// read the same.
static TITLE: &[u8] = b"CHAT";

/// The title band's right half: who the NEXT question goes to. A fact
/// about the panel, where the file browser puts its cwd — and the
/// band is also the toggle: a click on it cycles the pair.
static BAND_CLAUDE: &[u8] = b"CLAUDE";
static BAND_LOCAL: &[u8] = b"LOCAL";

/// The band's right half while the daemon is away.
static SAY_OFFLINE_BAND: &[u8] = b"offline";

/// The client name on the wire. Lowercase like the addon's file stem:
/// `aichat.so`, `client":"aichat"`.
const CLIENT: &str = "aichat";

/// What the empty box invites.
const PLACEHOLDER: &str = "ask, and Enter sends";

/// The empty state, when there is no daemon to ask. It names the thing
/// that is missing and promises no date, for the same reason the old AI
/// panel's message did: this file has no way of knowing one.
const SAY_OFFLINE: &str =
    "Waiting for the nacelle-ai daemon. Chat takes no input until that daemon answers.";

/// The approval row's captions, as content. The CASE they draw in is
/// the button role's `case` token's decision, not these strings'.
const CAP_ALLOW: &str = "allow";
const CAP_DENY: &str = "deny";

/// The progress row's stand-in while a request runs and the daemon has
/// not said a sentence about it yet.
const SAY_WORKING: &str = "working\u{2026}";

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

/// The key event a boundary call means, if any — the search panel's
/// reading, byte for byte: [`keys::from_name`] is the ABI's own
/// spelling of the neutral key set, unknown modifier bits are dropped
/// rather than kept, and a control character is no key at all.
fn key_ev(ch: u32, label: Option<&str>, mods: u32) -> Option<KeyEv> {
    let key = match label {
        Some(l) => keys::from_name(l)?,
        None => {
            let c = char::from_u32(ch).filter(|c| !c.is_control())?;
            Key::Char(c)
        }
    };
    Some(KeyEv {
        key,
        mods: Mods::from_bits((mods & 0xff) as u8),
        repeat: false,
        text: None,
    })
}

/// The label a boundary call passed, as a string — or `None` for a
/// character key, which is what an empty one means on both key entries.
///
/// # Safety
/// `label` must either be null or point at `label_len` readable bytes
/// that stay put for the call — the key entries' own contract.
unsafe fn label_of<'a>(label: *const u8, label_len: u32) -> Option<&'a str> {
    if label.is_null() || label_len == 0 {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(label, label_len as usize)).ok()
}

// ------------------------------------------------------------------ look

/// The case transform a role binding's `case` token asks for, as its
/// word. Applied here because [`Surface::text`] draws bytes as given;
/// smallcaps needs per-glyph sizes only the host has, so through a
/// single text call the nearest honest reading is capitals.
fn role_case(sf: &mut impl Surface, binding: &str) -> String {
    let role = sf.word(binding);
    if role.is_empty() {
        return String::new();
    }
    sf.word(&format!("type.{role}.case"))
}

fn recase(word: &str, s: &str) -> String {
    match word {
        "upper" | "smallcaps" => s.to_uppercase(),
        "lower" => s.to_lowercase(),
        _ => s.to_string(),
    }
}

/// One button: the `button` class's rung for its state, under the
/// `button.*` shape tokens and the role `button.role` binds — the same
/// names the CONTROL panel and the AI tool panels draw theirs from, so
/// the interface's buttons are one object and not a family of
/// lookalikes.
fn button(sf: &mut impl Surface, r: Rect, caption: &str, state: State) {
    let corner = paint::corner_radius(sf, "button.corner", r, 1.0);
    let cut = paint::corner_style(sf, "button.corner_style");
    let ink = sf.class_state("button", state);
    sf.ring_fill(r, cut, corner, ink.fill);
    if ink.edge_width > 0.0 {
        sf.ring(r, cut, corner, ink.edge_width, ink.edge);
    }
    let role = paint::bound_role(sf, "button.role", 1.0);
    if role.px <= 0.0 {
        return;
    }
    let case = role_case(sf, "button.role");
    let cap = recase(&case, caption);
    let ty = paint::center_line_y(sf, r.y, r.h, role.px, role.leading);
    sf.text(role.face, role.px, r.cx(), ty, &cap, ink.text, role.track, Align::Center);
}

/// The width a button asks for: its caption at the button role, plus
/// the same inset a field gives its own text on either side. No length
/// of this file's own — `field.pad_x` is the master's word for "the
/// breathing room around one line in a box".
fn button_w(sf: &mut impl Surface, caption: &str) -> f32 {
    let role = paint::bound_role(sf, "button.role", 1.0);
    let case = role_case(sf, "button.role");
    let cap = recase(&case, caption);
    let pad = sf.px("field.pad_x").max(0.0);
    sf.measure(role.face, role.px, &cap, role.track) + 2.0 * pad
}

/// The message broken to fit `max_w`, greedily, at spaces — the old AI
/// panel's wrap, carried for the transcript it promised. A line always
/// takes its first word however narrow the box is, so a `max_w` that is
/// zero or nonsense gives one word per line rather than a loop that
/// does not end.
fn wrap(message: &str, max_w: f32, mut width_of: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut probe = String::new();
    for word in message.split_whitespace() {
        match out.last_mut() {
            Some(line) => {
                probe.clear();
                probe.push_str(line);
                probe.push(' ');
                probe.push_str(word);
                if width_of(&probe) <= max_w {
                    line.push(' ');
                    line.push_str(word);
                } else {
                    out.push(word.to_string());
                }
            }
            None => out.push(word.to_string()),
        }
    }
    out
}

// ------------------------------------------------------------- transcript

/// One drawn line of the transcript: whose turn it belongs to, and the
/// text of this line. Empty text is a paragraph break an answer wrote —
/// the row keeps its height and the text call is skipped.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    who: Who,
    text: String,
}

/// What the cached wrap was computed against. The stamp is the model's
/// own invalidation key; the rest is everything else the wrap reads —
/// the theme epoch, the text width, and the two roles' sizes (which a
/// resize can move without the epoch moving).
type LinesKey = (u64, u32, u32, u32, u32);

/// The role a turn is set in. The user's own words carry the list's
/// label role and everything the daemon or the panel says carries the
/// status role — content and commentary, told apart the way every list
/// in this tree tells them apart, so a theme that styles one styles
/// this transcript with it.
fn role_of<'a>(who: Who, label: &'a RoleLook, status: &'a RoleLook) -> &'a RoleLook {
    match who {
        Who::You => label,
        Who::Ai | Who::Note => status,
    }
}

/// The transcript rewrapped to `w`: each turn split at its own
/// newlines, each paragraph wrapped at the width, in its turn's role.
fn relayout(
    sf: &mut impl Surface,
    turns: &[Turn],
    label: &RoleLook,
    status: &RoleLook,
    w: f32,
    out: &mut Vec<Line>,
) {
    out.clear();
    for turn in turns {
        let role = role_of(turn.who, label, status);
        if role.px <= 0.0 {
            // A role the theme said nothing about draws nothing, and a
            // wrap for it would be work with no reader.
            continue;
        }
        for para in turn.text.split('\n') {
            if para.split_whitespace().next().is_none() {
                // A blank line the answer wrote: kept as a row, drawn as
                // nothing.
                out.push(Line { who: turn.who, text: String::new() });
                continue;
            }
            for text in wrap(para, w, |s| sf.measure(role.face, role.px, s, role.track)) {
                out.push(Line { who: turn.who, text });
            }
        }
    }
}

// ------------------------------------------------------------- the widget

/// A physics before the first frame has read one: everything zero, so a
/// wheel that arrives before a draw moves nothing — the same raw answer
/// a missing token would give.
const NO_PHYSICS: ScrollPhysics = ScrollPhysics {
    wheel_px: 0.0,
    fling_scale: 0.0,
    glide_halflife_ms: 0.0,
    settle_ms: 0.0,
    settle_easing: Easing::Linear,
    motion_scale: 0.0,
};

const NO_RECT: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

pub struct AiChat {
    /// The model: the prompt, the transcript, the flight, the toggle.
    model: Chat,
    /// The prompt box's between-frame state.
    field: FieldView,
    /// The daemon's client: one non-blocking socket, polled once per
    /// frame from the draw path, Offline until it answers.
    client: AiClient,
    /// The transcript's scroll — the toolkit's own view, with the
    /// toolkit's own physics.
    scroll: ScrollView,
    /// What the last frame drew, for the input that arrives with no
    /// geometry of its own. Zeroed while the daemon is away, so an
    /// offline panel's clicks land on nothing — the controls are not
    /// there, and neither are their rectangles.
    content_r: Rect,
    field_r: Rect,
    allow_r: Rect,
    deny_r: Rect,
    /// The bar the last frame drew: track, thumb, and the viewport it
    /// reported — a click on the track pages by exactly that height.
    bar: Option<(Rect, Rect, f32)>,
    /// The physics and the clock the last draw read, cached because a
    /// wheel event arrives with no drawing context to ask the theme
    /// through — the file browser's own arrangement.
    physics: ScrollPhysics,
    frame_t: f64,
    /// The wrapped transcript and what it was wrapped against. The
    /// model's `stamp` is the key's moving part: a frame where nothing
    /// stamped re-uses the wrap instead of measuring two hundred turns
    /// again.
    lines: Vec<Line>,
    lines_key: Option<LinesKey>,
    /// Whether the view sat at the bottom of the transcript last frame.
    /// While it does, a streamed delta keeps it there; the moment the
    /// reader scrolls away, the stream stops moving the view.
    follow: bool,
    /// Whether the last poll saw the daemon, so the frame after a
    /// hang-up can tell the model its flight died with the socket.
    was_connected: bool,
}

impl AiChat {
    pub fn new() -> AiChat {
        AiChat::with_client(AiClient::new(CLIENT))
    }

    /// The same widget over an explicit client — the seam the tests
    /// build through, aimed at a socket path of their own.
    pub fn with_client(client: AiClient) -> AiChat {
        AiChat {
            model: Chat::new(),
            field: FieldView::new(),
            client,
            scroll: ScrollView::new(),
            content_r: NO_RECT,
            field_r: NO_RECT,
            allow_r: NO_RECT,
            deny_r: NO_RECT,
            bar: None,
            physics: NO_PHYSICS,
            frame_t: 0.0,
            lines: Vec::new(),
            lines_key: None,
            // A transcript nobody has scrolled follows the stream: the
            // first answer is watched from its first delta.
            follow: true,
            was_connected: false,
        }
    }

    /// One frame's worth of daemon: knock or read, then fold every
    /// event that arrived into the model. The draw path is the pump —
    /// the client counts polls, not seconds, so an occluded panel costs
    /// nothing and knocks at nothing.
    fn pump(&mut self) {
        self.client.poll();
        while let Some(ev) = self.client.take_event() {
            self.model.on_event(ev);
        }
        let connected = self.client.status() == Status::Connected;
        if self.was_connected && !connected {
            self.model.connection_lost();
        }
        self.was_connected = connected;
    }

    /// Sends the prompt. The question joins the transcript either way:
    /// when the client is Offline after all — the race between the
    /// frame that drew Connected and the Enter — the transcript says
    /// what happened to it instead of leaving it to read as unanswered.
    fn send(&mut self) {
        let text = self.model.take_prompt();
        match self.client.ask(&text, self.model.backend()) {
            Some(id) => self.model.sent(id),
            None => self.model.not_sent(),
        }
    }

    /// The user answered the approval question. The id goes back over
    /// the same confidentiality line the question came over; a click
    /// that raced the daemon's own `error` has nothing left to answer
    /// and sends nothing.
    fn answer(&mut self, allow: bool) {
        if let Some(id) = self.model.take_approval() {
            self.client.approve(id, allow);
        }
    }

    /// The user cancelled the request in flight (Escape over an empty
    /// box).
    fn cancel(&mut self) {
        if let Some(id) = self.model.pending() {
            self.client.cancel(id);
        }
        self.model.cancelled();
    }

    /// The empty state: the offline message, wrapped, centred on
    /// `emptystate.y_frac` — the same reading every panel with nothing
    /// to show gives that token. A theme that declares no role draws
    /// nothing, which is what "no hardcoded values" degrades to.
    fn offline(&mut self, sf: &mut impl Surface, r: Rect) {
        let role = paint::bound_role(sf, "emptystate.role", 1.0);
        if role.px <= 0.0 || role.color.a <= 0.0 {
            return;
        }
        let case = role_case(sf, "emptystate.role");
        let message = recase(&case, SAY_OFFLINE);
        let y_frac = sf.px("emptystate.y_frac");
        let lines = wrap(&message, r.w, |s| sf.measure(role.face, role.px, s, role.track));
        let pitch = role.px * role.leading.max(1.0);
        let block = lines.len() as f32 * pitch;
        let top = (r.y + r.h * y_frac - block / 2.0).max(r.y);
        for (i, line) in lines.iter().enumerate() {
            let y = top + i as f32 * pitch;
            if y < r.y || y + pitch > r.bottom() {
                continue;
            }
            sf.text(role.face, role.px, r.cx(), y, line, role.color, role.track, Align::Center);
        }
    }

    /// The transcript: the wrapped turns as a scrolled column, with the
    /// toolkit's bar beside them. Adoption of `view::scroll`, not a
    /// second copy of its arithmetic: the physics, the snap, the bar's
    /// geometry and its fade are all the toolkit's own.
    fn transcript(&mut self, sf: &mut impl Surface, r: Rect) {
        self.bar = None;
        if r.h <= 0.0 || r.w <= 0.0 {
            return;
        }
        let label = paint::bound_role(sf, "list.label_role", 1.0);
        let status = paint::bound_role(sf, "list.status_role", 1.0);
        let row_h = sf.px("list.row_h").max(1.0);
        let gap = sf.px("list.gap").max(0.0);
        let pitch = row_h + gap;

        // An inset bar narrows the column the text wraps to — the
        // master's own word for "the bar takes room from the content".
        let look = ScrollbarLook::read(sf);
        let text_w = (r.w - scroll::inset_w(&look)).max(1.0);

        let key: LinesKey = (
            self.model.stamp(),
            sf.epoch(),
            text_w.to_bits(),
            label.px.to_bits(),
            status.px.to_bits(),
        );
        let moved = self.lines_key != Some(key);
        if moved {
            relayout(sf, self.model.turns(), &label, &status, text_w, &mut self.lines);
            self.lines_key = Some(key);
        }
        let content = self.lines.len() as f32 * pitch;
        let max = (content - r.h).max(0.0);

        // Follow the stream — but only from the bottom. `set_offset`
        // rather than a settle: a delta a frame is sixty settles a
        // second, and the view would never arrive.
        if moved && self.follow {
            self.scroll.set_offset(max);
        }
        self.scroll.tick(self.frame_t, r.h, content, Snap::Row(pitch), &self.physics);
        let offset = self.scroll.offset();
        self.follow = offset >= max - 0.5;

        // The visible rows, under a clip: a partial row at either edge
        // is cut by the box, never drawn over the neighbours.
        let clipped = sf.clip(r);
        let first = (offset / pitch).max(0.0) as usize;
        for i in first..self.lines.len() {
            let y = r.y + i as f32 * pitch - offset;
            if y >= r.bottom() {
                break;
            }
            let line = &self.lines[i];
            if line.text.is_empty() {
                continue;
            }
            let role = role_of(line.who, &label, &status);
            if role.px <= 0.0 {
                continue;
            }
            let ty = paint::center_line_y(sf, y, row_h, role.px, role.leading);
            sf.text(role.face, role.px, r.x, ty, &line.text, role.color, role.track, Align::Left);
        }
        if clipped {
            sf.unclip();
        }

        // The bar. Hover is asked of the WIDE bar, so it cannot shrink
        // out from under the pointer and flicker at the seam — the file
        // browser's own arrangement.
        let (mx, my) = sf.mouse();
        let wide = ScrollbarLook { w: look.w.max(look.w_hover), ..look };
        let hovered = scrollbar(r, &wide, offset, r.h, content, false)
            .is_some_and(|g| g.track.contains(mx, my));
        if let Some(g) = scrollbar(r, &look, offset, r.h, content, hovered) {
            // `scrollbar.auto_hide`: full while the view moves or the
            // pointer is on the bar, fading to nothing over
            // `scrollbar.fade_ms` afterwards — the view's own answer.
            let fade = if hovered {
                1.0
            } else {
                self.scroll.fade_alpha(self.frame_t, look.auto_hide, look.fade_ms)
            };
            if fade > 0.0 {
                let ink =
                    sf.class_state("scrollbar.thumb", if hovered { State::Hover } else { State::Idle });
                let fill = Color { a: ink.fill.a * fade, ..ink.fill };
                if fill.a > 0.0 {
                    sf.rect(g.thumb, fill);
                }
                if ink.edge_width > 0.0 && ink.edge.a > 0.0 {
                    let edge = Color { a: ink.edge.a * fade, ..ink.edge };
                    sf.rect_outline(g.thumb, ink.edge_width, edge);
                }
            }
            self.bar = Some((g.track, g.thumb, r.h));
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let mut sf = AbiSurface::new(api, ctx);
        self.pump();
        // Kept even while offline: the band above this box is where the
        // toggle lives, and `click` tells the band from the content by
        // this rect's top.
        self.content_r = r;

        if self.client.status() == Status::Offline {
            // No daemon, no controls, no rectangles: a click on where
            // anything was must land on nothing.
            self.field_r = NO_RECT;
            self.allow_r = NO_RECT;
            self.deny_r = NO_RECT;
            self.bar = None;
            self.offline(&mut sf, r);
            return;
        }

        self.physics = ScrollPhysics::read(&mut sf);
        self.frame_t = sf.now();
        let gap = sf.px("list.gap").max(0.0);
        let field_h = sf.px("field.h").max(0.0).min(r.h);

        // The prompt box, at the bottom: a chat reads down to where the
        // next word goes. As in the search panel, there is no focus
        // chain across the ABI, so the panel answers the question the
        // chain would: the box owns the keyboard while it is on screen.
        let field_r = Rect::new(r.x, r.bottom() - field_h, r.w, field_h);
        field::draw(&mut sf, field_r, &self.model.input, &mut self.field, PLACEHOLDER, true);
        self.field_r = field_r;

        // Above it: the question on the table, or the run's latest word.
        self.allow_r = NO_RECT;
        self.deny_r = NO_RECT;
        let mut strip_bottom = field_r.y - gap;
        let approval = self.model.approval().map(|(_, d)| d.to_string());
        if let Some(desc) = approval {
            let row_h = field_h.min((strip_bottom - r.y).max(0.0));
            let row_y = strip_bottom - row_h;
            if row_h > 0.0 {
                let (mx, my) = sf.mouse();
                let aw = button_w(&mut sf, CAP_ALLOW).min(r.w);
                let dw = button_w(&mut sf, CAP_DENY).min((r.w - aw - gap).max(0.0));
                let allow_r = Rect::new(r.x, row_y, aw, row_h);
                let deny_r = Rect::new(r.x + aw + gap, row_y, dw, row_h);
                let state = |b: &Rect| {
                    if b.contains(mx, my) {
                        State::Hover
                    } else {
                        State::Idle
                    }
                };
                button(&mut sf, allow_r, CAP_ALLOW, state(&allow_r));
                button(&mut sf, deny_r, CAP_DENY, state(&deny_r));
                self.allow_r = allow_r;
                self.deny_r = deny_r;
                // The daemon's description of what it wants to do, in
                // the row's remaining width — the question the two
                // buttons answer, kept beside them.
                let label = paint::bound_role(&mut sf, "list.label_role", 1.0);
                if label.px > 0.0 {
                    let tx = deny_r.right() + gap;
                    let tw = (r.right() - tx).max(0.0);
                    if tw > 0.0 {
                        let fit = paint::fit_end(&mut sf, label.face, label.px, &desc, tw, label.track);
                        let ty = paint::center_line_y(&mut sf, row_y, row_h, label.px, label.leading);
                        sf.text(label.face, label.px, tx, ty, &fit, label.color, label.track, Align::Left);
                    }
                }
                strip_bottom = row_y - gap;
            }
        } else if self.model.pending().is_some() {
            let row_h = sf.px("list.row_h").max(1.0).min((strip_bottom - r.y).max(0.0));
            let row_y = strip_bottom - row_h;
            if row_h > 0.0 {
                let status = paint::bound_role(&mut sf, "list.status_role", 1.0);
                if status.px > 0.0 {
                    let msg = self.model.progress().unwrap_or(SAY_WORKING).to_string();
                    let fit = paint::fit_end(&mut sf, status.face, status.px, &msg, r.w, status.track);
                    let ty = paint::center_line_y(&mut sf, row_y, row_h, status.px, status.leading);
                    sf.text(status.face, status.px, r.x, ty, &fit, status.color, status.track, Align::Left);
                }
                strip_bottom = row_y - gap;
            }
        }

        // The transcript, in whatever the rows above left.
        let strip_r = Rect::new(r.x, r.y, r.w, (strip_bottom - r.y).max(0.0));
        self.transcript(&mut sf, strip_r);
    }

    /// A press. The rectangles are the last frame's, because a click
    /// arrives with no geometry of its own — the search panel's rule.
    ///
    /// Above the content box is the panel's title band, and the band is
    /// the backend toggle: CLAUDE ⇄ LOCAL, cycled per click, exactly as
    /// the band's right half names it. Only while the daemon is there —
    /// offline, the band says `offline` and there is nothing to toggle.
    pub fn click(&mut self, x: f32, y: f32) {
        if self.content_r.h > 0.0 && y < self.content_r.y {
            if self.client.status() == Status::Connected {
                self.model.cycle_backend();
            }
            return;
        }
        if self.field_r.contains(x, y) {
            let at = self.field.hit(x);
            self.model
                .input
                .apply(nacelle::object::text_input::InputMsg::Point { at, extend: false });
            return;
        }
        if self.allow_r.contains(x, y) {
            self.answer(true);
            return;
        }
        if self.deny_r.contains(x, y) {
            self.answer(false);
            return;
        }
        if let Some((track, thumb, viewport)) = self.bar {
            // The track beside the thumb: one viewport toward the
            // click — the toolkit's own page, not arithmetic invented
            // here.
            if track.contains(x, y) && !thumb.contains(x, y) {
                self.scroll.page(y > thumb.y, viewport, self.frame_t);
            }
        }
    }

    /// The wheel, over the transcript. The physics are the last draw's,
    /// because a wheel event arrives with no drawing context to ask the
    /// theme through. The sign is the host's own: positive `dy` is
    /// toward the top of the content, as every scrolled panel here
    /// reads it.
    pub fn wheel(&mut self, dy: f32) {
        let p = self.physics;
        self.scroll.wheel(-dy, &p, self.frame_t);
    }

    /// A key delivered to this panel. Answers whether it was consumed.
    pub fn key(&mut self, ev: &KeyEv) -> bool {
        match self.model.key(ev) {
            Outcome::Submit => {
                self.send();
                true
            }
            Outcome::CancelPending => {
                self.cancel();
                true
            }
            Outcome::Edited | Outcome::Moved => true,
            Outcome::Ignored => false,
        }
    }
}

impl Default for AiChat {
    fn default() -> Self {
        AiChat::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(AiChat::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut AiChat) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut AiChat> {
    unsafe { (instance as *mut AiChat).as_mut() }
}

extern "C" fn draw_c(
    instance: *mut c_void,
    ctx: *mut c_void,
    _host_data: *const c_void,
    r: RectC,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    this.draw(api, ctx, Rect::new(r.x, r.y, r.w, r.h));
}

extern "C" fn click_c(
    instance: *mut c_void,
    x: f32,
    y: f32,
    _r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    if let Some(this) = state(instance) {
        this.click(x, y);
    }
    if let Some(out) = unsafe { out.as_mut() } {
        // Whatever was pressed is already on its way to the daemon; the
        // host has nothing to do about it.
        out.kind = ACTION_NONE;
    }
}

/// The wheel scrolls the transcript — through the toolkit's own
/// `view::scroll`, with the physics the last draw read.
extern "C" fn wheel_c(
    instance: *mut c_void,
    dy: f32,
    _r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    if let Some(this) = state(instance) {
        this.wheel(dy);
    }
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = ACTION_NONE;
    }
}

extern "C" fn grid_c(_: *mut c_void, _: *mut u32, _: *mut u32) {}

/// Whether this host routes keys to the widget that owns the keyboard —
/// the search panel's probe, for the search panel's reason: on a host
/// that routes, acting on the broadcast too would type every character
/// twice.
fn routes_keys(api: &HostApi) -> bool {
    api.has_channel()
}

/// The broadcast path, kept only for a host too old to route keys: no
/// modifiers cross it, so the field's chords stay out of reach there.
extern "C" fn key_feedback_c(
    instance: *mut c_void,
    ch: u32,
    label: *const u8,
    label_len: u32,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    if routes_keys(api) {
        return;
    }
    // Safety: the entry's contract — a null pointer or `label_len`
    // readable bytes that outlive the call.
    let label = unsafe { label_of(label, label_len) };
    if let Some(ev) = key_ev(ch, label, 0) {
        this.key(&ev);
    }
}

/// The key delivered to the widget that owns the keyboard, modifiers
/// and all. Nonzero says the panel used it, so the host must not spend
/// it again on focus navigation or a shortcut.
extern "C" fn key_c(
    instance: *mut c_void,
    ch: u32,
    label: *const u8,
    label_len: u32,
    mods: u32,
    out: *mut ActionC,
) -> u32 {
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = ACTION_NONE;
    }
    let Some(this) = state(instance) else { return 0 };
    // Safety: the entry's contract, as above.
    let label = unsafe { label_of(label, label_len) };
    let Some(ev) = key_ev(ch, label, mods) else { return 0 };
    this.key(&ev) as u32
}

/// Sized against the reference box: what grows with a taller panel is
/// the transcript, not the size of anything in it.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    SIZING_REFERENCE
}

/// The header: the panel's name, and the right half the old AI panel's
/// own file promised — who answers. `CLAUDE` or `LOCAL` while the
/// daemon is there (the band is also the toggle: `click` cycles it),
/// `offline` while it is not.
extern "C" fn chrome_c(
    instance: *mut c_void,
    _ctx: *mut c_void,
    _host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return 0;
    };
    out.title = TITLE.as_ptr();
    out.title_len = TITLE.len() as u32;
    let band: &'static [u8] = if this.client.status() == Status::Offline {
        SAY_OFFLINE_BAND
    } else {
        match this.model.backend() {
            Backend::Local => BAND_LOCAL,
            _ => BAND_CLAUDE,
        }
    };
    out.right = band.as_ptr();
    out.right_len = band.len() as u32;
    (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
}

/// This widget takes no drags: the thumb is paged and wheeled, not yet
/// held (the toolkit's `press_thumb`/`drag` wait on the press phase
/// carrying capture across this ABI). Declining every Begin keeps a
/// press on the ordinary click path, which is where the buttons are.
#[allow(clippy::too_many_arguments)]
extern "C" fn drag_c(
    _: *mut c_void,
    _: u32,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
    _: *mut ActionC,
) {
}

/// No hand cursor: the buttons speak through their hover rung, and the
/// panel keeps the ordinary pointer.
extern "C" fn pointer_c(
    _: *mut c_void,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
) -> u32 {
    0
}

/// Filled, and does nothing on purpose: the buttons answer the CLICK,
/// which is press-and-release in one word, and a press rung this file
/// drew from its own guess at the gesture would disagree with the
/// host's. When the press phase carries something this panel wants —
/// the thumb grab above — it arrives here and gets a body.
#[allow(clippy::too_many_arguments)]
extern "C" fn button_c(
    _: *mut c_void,
    _: u32,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
    _: *mut ActionC,
) {
}

static API: PluginApi = PluginApi {
    abi_version: ABI_VERSION,
    api_size: std::mem::size_of::<PluginApi>() as u32,
    create,
    destroy,
    draw: draw_c,
    click: click_c,
    wheel: wheel_c,
    grid: grid_c,
    key_feedback: key_feedback_c,
    sizing,
    chrome: chrome_c,
    drag: drag_c,
    pointer: pointer_c,
    key: key_c,
    button: button_c,
};

/// This addon, for a host that LINKS the crate in instead of loading
/// `aichat.so` from the addons directory. The name and the metadata are
/// the addon's own — the same string the file would be called and the
/// very bytes of `aichat.meta` beside it — so a host never describes a
/// widget it merely links.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "aichat",
    meta: include_str!("../aichat.meta"),
    attach: builtin_attach,
};

/// In-process attach for a host that links this crate statically. The
/// dlopen attach below goes through `runtime::attach`, which flips the
/// toolkit into forwarding mode — correct for a plugin carrying its own
/// copy of the toolkit, and exactly wrong when this copy IS the host's.
pub fn builtin_attach(api: &'static HostApi) -> *const PluginApi {
    unsafe { HOST = Some(api) };
    &API
}

/// # Safety
/// Called by the host with its own interface, once, before anything
/// else. `api` must point at a `HostApi` the host keeps alive for the
/// life of the program.
#[cfg(feature = "dyn")]
#[no_mangle]
pub unsafe extern "C" fn nacelle_plugin_attach(api: *const HostApi) -> *const PluginApi {
    if !nacelle::runtime::attach(api) {
        return std::ptr::null();
    }
    HOST = api.as_ref();
    &API
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod meta_tests {
    use super::*;
    use nacelle::base::WidgetCategory;
    use nacelle::widget::registry;

    /// The widget declares itself onto the top fixture board, and the
    /// declaration the host will read is the one this crate carries:
    /// the linked-in constant `include_str!`s the very file the
    /// installer copies, so the two cannot say different things.
    #[test]
    fn the_widget_registers_on_the_search_and_ai_board() {
        let def = registry::def_from_meta(WIDGET.name.to_string(), WIDGET.meta);
        assert_eq!(def.name, "aichat");
        assert_eq!(def.label, "CHAT");
        assert_eq!(def.category, WidgetCategory::SearchAi);
        // The board is asked for explicitly and never fallen into: an
        // unknown or absent category word is a BOARD widget, so a typo
        // here would silently put the panel on the wrong board.
        assert_ne!(WidgetCategory::default(), WidgetCategory::SearchAi);
        assert!(def.ref_h_vh > 0.0 && def.min_h_vh > 0.0);
        assert!(def.min_h_vh <= def.ref_h_vh);
    }
}

#[cfg(test)]
mod token_tests {
    /// Every token this crate names, spelled exactly as the code spells
    /// it — the prompt box, the buttons, the transcript, its scroll and
    /// its bar, and the empty state.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise: a widget that names a token the master does not
    /// declare degrades silently to nothing drawn, so a typo fails
    /// nowhere else — and therefore fails here.
    const TOKENS: &[&str] = &[
        // the prompt box — [field] and its component colours
        "field.h",
        "field.corner",
        "field.corner_style",
        "field.pad_x",
        "field.border",
        "field.border_focused",
        "field.role",
        "field.scroll_margin",
        "field.caret_w",
        "field.caret_h",
        "field.caret_style",
        "field.preedit_underline",
        "component.field.fill",
        "component.field.border",
        "component.field.text",
        "component.field.placeholder",
        "component.field.caret",
        "component.field.selection",
        "component.field.selection_text",
        "component.field.preedit",
        // the caret's blink
        "motion.scale",
        "motion.caret_blink.enabled",
        "motion.caret_blink.period_ms",
        "motion.caret_blink.duty",
        // the buttons — the same names CONTROL draws from
        "button.corner",
        "button.corner_style",
        "button.role",
        // the transcript's rows and roles
        "list.row_h",
        "list.gap",
        "list.label_role",
        "list.status_role",
        // its scroll — the toolkit's own physics
        "scroll.wheel_px",
        "scroll.fling_scale",
        "scroll.glide_halflife_ms",
        "motion.scroll_settle.enabled",
        "motion.scroll_settle.duration_ms",
        "motion.scroll_settle.easing",
        "motion.scroll_settle.duty",
        "motion.scroll_settle.floor",
        // and its bar
        "scrollbar.mode",
        "scrollbar.edge",
        "scrollbar.w",
        "scrollbar.w_hover",
        "scrollbar.margin",
        "scrollbar.thumb_min",
        "scrollbar.auto_hide",
        "scrollbar.fade_ms",
        // and the panel with no daemon to talk to
        "emptystate.role",
        "emptystate.y_frac",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
        // The classes the controls rest on. A class the matrix does not
        // know answers the raw rung, and the panel would look
        // undesigned for a reason no reader could see.
        for class in ["field", "button", "scrollbar.thumb"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// A stand-in for the host's font measurement: every character the
    /// same width. What matters to the wrap is that a longer string is
    /// wider, which is the only property it uses.
    fn width_of(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    /// A role as the theme would answer one: the resolver's own no-role
    /// with a size — nothing else in it is read by the wrap.
    fn role(px: f32) -> RoleLook {
        RoleLook { px, ..paint::NO_ROLE }
    }

    /// A probe surface for the relayout: measurement only, everything
    /// else inert — the wrap is arithmetic, and arithmetic needs no
    /// window.
    struct Ruler;
    impl Surface for Ruler {
        fn ring_fill(&mut self, _: Rect, _: nacelle::draw::CornerStyle, _: f32, _: Color) {}
        fn ring(&mut self, _: Rect, _: nacelle::draw::CornerStyle, _: f32, _: f32, _: Color) {}
        fn rect(&mut self, _: Rect, _: Color) {}
        fn rect_outline(&mut self, _: Rect, _: f32, _: Color) {}
        fn line(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: Color) {}
        fn polyline(&mut self, _: &[[f32; 2]], _: f32, _: Color, _: bool) {}
        #[allow(clippy::too_many_arguments)]
        fn text(&mut self, _: u8, _: f32, _: f32, _: f32, _: &str, _: Color, _: f32, _: Align) {}
        fn measure(&mut self, _: u8, _: f32, s: &str, _: f32) -> f32 {
            width_of(s)
        }
        fn clip(&mut self, _: Rect) -> bool {
            false
        }
        fn unclip(&mut self) {}
        fn has_token(&mut self, _: &str) -> bool {
            false
        }
        fn px(&mut self, _: &str) -> f32 {
            0.0
        }
        fn color(&mut self, _: &str) -> Color {
            Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
        }
        fn bed(&mut self, _: &str) -> Color {
            Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
        }
        fn flag(&mut self, _: &str) -> bool {
            false
        }
        fn word(&mut self, _: &str) -> String {
            String::new()
        }
        fn class_state(&mut self, _: &str, _: State) -> nacelle::view::surface::StateInk {
            nacelle::view::surface::StateInk::raw()
        }
        fn epoch(&mut self) -> u32 {
            0
        }
        fn now(&self) -> f64 {
            0.0
        }
        fn mouse(&self) -> (f32, f32) {
            (-1.0, -1.0)
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    /// The transcript's wrap: every word of every turn survives, in
    /// order, each line no wider than the column — first-word lines
    /// excepted, which is the wrap refusing to loop forever.
    #[test]
    fn the_transcript_wraps_every_turn_and_loses_no_word() {
        let turns = vec![
            Turn { who: Who::You, text: "why is the sky the colour it is".into() },
            Turn { who: Who::Ai, text: "scattering, mostly.\n\nRayleigh's kind".into() },
            Turn { who: Who::Note, text: "cancelled".into() },
        ];
        let mut lines = Vec::new();
        relayout(&mut Ruler, &turns, &role(16.0), &role(16.0), 120.0, &mut lines);
        assert!(lines.len() > 3, "a 12-character column must wrap these turns");
        // The blank paragraph the answer wrote is a row of its own.
        assert!(lines.iter().any(|l| l.text.is_empty() && l.who == Who::Ai));
        for l in &lines {
            let words = l.text.split_whitespace().count();
            assert!(words <= 1 || width_of(&l.text) <= 120.0);
        }
        // Nothing lost, nothing reordered, and every line knows whose
        // turn it came from.
        let said: Vec<&str> = lines
            .iter()
            .filter(|l| l.who == Who::Ai && !l.text.is_empty())
            .flat_map(|l| l.text.split_whitespace())
            .collect();
        assert_eq!(said.join(" "), "scattering, mostly. Rayleigh's kind");
        assert!(lines.iter().all(|l| l.who != Who::You || !l.text.contains("scattering")));
    }

    /// A role the theme never spoke about wraps nothing: no size means
    /// nothing drawn, so measuring for it would be work with no reader.
    #[test]
    fn a_silent_role_wraps_to_nothing() {
        let turns = vec![Turn { who: Who::You, text: "hello".into() }];
        let mut lines = Vec::new();
        relayout(&mut Ruler, &turns, &role(0.0), &role(16.0), 120.0, &mut lines);
        assert!(lines.is_empty());
    }

    /// The wrap itself survives nonsense widths — the old AI panel's
    /// guarantee, kept for the transcript that replaced it.
    #[test]
    fn a_nonsense_width_gives_one_word_per_line_not_a_hang() {
        for w in [0.0, -100.0, f32::NAN] {
            let lines = wrap("three little words", w, width_of);
            assert_eq!(lines.len(), 3);
        }
        assert!(wrap("", 100.0, width_of).is_empty());
    }
}

#[cfg(test)]
mod offline_tests {
    use super::*;
    use std::path::PathBuf;

    /// A widget whose daemon does not exist: aimed at a socket path
    /// under a directory that is not there, so connect fails without
    /// touching anything real.
    fn stranded() -> AiChat {
        AiChat::with_client(AiClient::at(CLIENT, PathBuf::from("/nonexistent-aichat/ai.sock")))
    }

    /// Offline, the controls are NOT THERE — not merely unresponsive:
    /// the rectangles a click is answered from are zeroed, so a click
    /// anywhere lands on nothing and changes nothing — including the
    /// band's toggle, which has nothing to choose between while nobody
    /// answers.
    #[test]
    fn offline_takes_no_click_and_toggles_nothing() {
        let mut w = stranded();
        w.client.poll();
        assert_eq!(w.client.status(), Status::Offline);
        // The band's region, were the panel drawn: above the content
        // box. Cycling while offline would promise a choice the panel
        // cannot honour.
        w.content_r = Rect::new(0.0, 30.0, 300.0, 200.0);
        w.click(10.0, 10.0);
        assert_eq!(w.model.backend(), Backend::Claude, "offline, the toggle is not there");
        w.click(150.0, 100.0);
        assert_eq!(w.model.turns().len(), 0);
        assert_eq!(w.model.pending(), None);
    }

    /// Enter while Offline: the prompt is taken — it is already the
    /// user's words — and the transcript says it went nowhere, instead
    /// of the panel swallowing the question or pretending to ask it.
    #[test]
    fn enter_while_offline_is_said_in_the_transcript() {
        let mut w = stranded();
        w.client.poll();
        for c in "hello?".chars() {
            let ev = key_ev(c as u32, None, 0).unwrap();
            assert!(w.key(&ev));
        }
        let enter = key_ev(0, Some("ENTER"), 0).unwrap();
        assert!(w.key(&enter), "the Enter was this panel's: it moved the transcript");
        assert_eq!(w.model.pending(), None, "nothing is in flight toward a daemon that is not there");
        let turns = w.model.turns();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0], Turn { who: Who::You, text: "hello?".into() });
        assert_eq!(turns[1].who, Who::Note);
    }

    /// The wheel before any frame has drawn: the physics are the raw
    /// zeros, so nothing moves and nothing panics — polled forever, a
    /// daemonless panel stays exactly where it is.
    #[test]
    fn the_wheel_without_a_frame_moves_nothing() {
        let mut w = stranded();
        for _ in 0..10 {
            w.wheel(3.0);
            w.wheel(-3.0);
        }
        assert_eq!(w.scroll.offset(), 0.0);
    }

    /// The chrome while offline: the title, and the one fact worth the
    /// band — through the C entry, exactly as the host reads it.
    #[test]
    fn the_band_says_offline_while_the_daemon_is_away() {
        let mut w = stranded();
        w.client.poll();
        let mut chrome = ChromeC::empty();
        let n = chrome_c(
            (&mut w as *mut AiChat) as *mut c_void,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut chrome,
            std::mem::size_of::<ChromeC>() as u32,
        );
        assert!(n > 0);
        let title =
            unsafe { std::slice::from_raw_parts(chrome.title, chrome.title_len as usize) };
        assert_eq!(title, b"CHAT");
        let right =
            unsafe { std::slice::from_raw_parts(chrome.right, chrome.right_len as usize) };
        assert_eq!(right, b"offline");
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::time::Duration;

    /// A daemon's chair for one test: a real Unix socket in the scratch
    /// tmp, so the panel's whole road — command out, event in — runs
    /// over the transport it ships on, not a mock of it.
    fn chair(name: &str) -> (UnixListener, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nacelle-aichat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch dir for the socket");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind the test socket");
        (listener, path)
    }

    /// The panel against a live socket: hello on connect; the band
    /// click cycles the toggle; Enter sends the ask under the toggled
    /// backend, streamed events land in the transcript, the approval
    /// question is answered back under its own id. Every command on the
    /// wire is one a user action caused.
    #[test]
    fn the_panel_speaks_v0_end_to_end() {
        let (listener, path) = chair("e2e.sock");
        let mut w = AiChat::with_client(AiClient::at(CLIENT, path.clone()));
        w.pump();
        assert_eq!(w.client.status(), Status::Connected);
        let (peer, _) = listener.accept().expect("the client knocked");
        peer.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut reader = BufReader::new(peer.try_clone().unwrap());

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), r#"{"cmd":"hello","client":"aichat","proto":0}"#);

        // The band is above the content box; a click there cycles the
        // toggle, and ONLY the toggle — no line goes out for it.
        w.content_r = Rect::new(0.0, 30.0, 300.0, 200.0);
        w.click(10.0, 10.0);
        assert_eq!(w.model.backend(), Backend::Local);

        // Enter over a typed question: the one ask, under the toggled
        // backend, escaped by the client's own spelling.
        w.model.input.set_value("why?");
        let enter = key_ev(0, Some("ENTER"), 0).unwrap();
        assert!(w.key(&enter));
        assert_eq!(w.model.pending(), Some(1));
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), r#"{"cmd":"ask","id":1,"text":"why?","backend":"local"}"#);

        // The stream: deltas grow one live answer turn; progress is the
        // row over the prompt; the approval question waits for a click.
        let mut daemon = peer.try_clone().unwrap();
        daemon
            .write_all(
                concat!(
                    "{\"ev\":\"delta\",\"id\":1,\"text\":\"because \"}\n",
                    "{\"ev\":\"delta\",\"id\":1,\"text\":\"physics\"}\n",
                    "{\"ev\":\"progress\",\"id\":1,\"msg\":\"checking\"}\n",
                    "{\"ev\":\"approval\",\"id\":1,\"desc\":\"read a file\"}\n",
                )
                .as_bytes(),
            )
            .unwrap();
        daemon.flush().unwrap();
        // The pump is the draw path's; a test frame calls it directly.
        // The socket owes no schedule, so knock until the bytes landed.
        for _ in 0..200 {
            w.pump();
            if w.model.approval().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            w.model.turns().last(),
            Some(&Turn { who: Who::Ai, text: "because physics".into() })
        );
        assert_eq!(w.model.progress(), Some("checking"));
        assert_eq!(w.model.approval(), Some((1, "read a file")));

        // ALLOW, as the click entry would answer it: the approve line
        // goes back under the question's own id.
        w.answer(true);
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim_end(), r#"{"cmd":"approve","id":1,"allow":true}"#);
        assert_eq!(w.model.approval(), None);

        // Done closes the flight; the transcript keeps the answer.
        daemon.write_all(b"{\"ev\":\"done\",\"id\":1}\n").unwrap();
        for _ in 0..200 {
            w.pump();
            if w.model.pending().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(w.model.pending(), None);
        assert_eq!(w.model.turns().len(), 2, "you, and the answer");

        drop(reader);
        drop(daemon);
        drop(peer);
        let _ = std::fs::remove_file(&path);
    }

    /// The daemon hanging up mid-flight: the next pump notices, the
    /// flight is failed in the transcript's own words, and the panel is
    /// Offline — a state, never a panic.
    #[test]
    fn a_hangup_mid_flight_is_said_and_survived() {
        let (listener, path) = chair("gone.sock");
        let mut w = AiChat::with_client(AiClient::at(CLIENT, path.clone()));
        w.pump();
        assert_eq!(w.client.status(), Status::Connected);
        let (peer, _) = listener.accept().unwrap();
        w.model.input.set_value("q");
        let enter = key_ev(0, Some("ENTER"), 0).unwrap();
        assert!(w.key(&enter));
        assert_eq!(w.model.pending(), Some(1));
        // The daemon dies with the answer half-owed.
        drop(peer);
        drop(listener);
        for _ in 0..200 {
            w.pump();
            if w.client.status() == Status::Offline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(w.client.status(), Status::Offline);
        assert_eq!(w.model.pending(), None, "the flight died with the socket");
        assert_eq!(
            w.model.turns().last().map(|t| t.who),
            Some(Who::Note),
            "and the transcript says so"
        );
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use nacelle::runtime::{
        BUTTON_PRESS, BUTTON_RELEASE, DRAG_BEGIN, DRAG_END, DRAG_MOVE, PLUGIN_API_HAS_BUTTON,
    };

    /// A value no entry of this widget could ever write, so "left
    /// alone" is something a test can see.
    fn untouched() -> ActionC {
        ActionC { kind: u32::MAX, index: 0, lines: 0, data: std::ptr::null(), data_len: 0 }
    }

    /// The inputs this widget does not use are INERT, and pinned so:
    /// the drag captures nothing, the pointer asks for no cursor, the
    /// press rung writes nothing, the grid reports no cells. Each is a
    /// decision written above its entry, and a change that gives one a
    /// body has to come past this test rather than around it.
    #[test]
    fn the_inputs_this_widget_does_not_use_are_inert() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);

        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        // The drag: every phase declined, no capture ever asked for.
        let mut a = untouched();
        for phase in [DRAG_BEGIN, DRAG_MOVE, DRAG_END] {
            (API.drag)(std::ptr::null_mut(), phase, 1.0, 1.0, r, 100.0, 100.0, &mut a);
        }
        assert_eq!(a.kind, u32::MAX, "a drag entry that does nothing writes nothing");

        // The pointer: no hand cursor anywhere.
        assert_eq!((API.pointer)(std::ptr::null_mut(), 1.0, 1.0, r, 100.0, 100.0), 0);

        // The press rung: filled, declared, and deliberately empty.
        let mut b = untouched();
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(std::ptr::null_mut(), phase, 1.0, 1.0, r, 100.0, 100.0, &mut b);
        }
        assert_eq!(b.kind, u32::MAX, "a press entry that does nothing writes nothing");

        // The grid: this widget has no cells to report.
        let (mut cols, mut rows) = (u32::MAX, u32::MAX);
        (API.grid)(std::ptr::null_mut(), &mut cols, &mut rows);
        assert_eq!((cols, rows), (u32::MAX, u32::MAX));
    }

    /// The wheel and the click ARE used — and both answer the host with
    /// "nothing for you to do", on a live instance and on a null one.
    #[test]
    fn the_used_entries_answer_and_ask_nothing_of_the_host() {
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let inst = (API.create)();
        assert!(!inst.is_null());
        let mut a = untouched();
        (API.wheel)(inst, 3.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_NONE);
        let mut b = untouched();
        (API.click)(inst, 50.0, 50.0, r, 100.0, 100.0, &mut b);
        assert_eq!(b.kind, ACTION_NONE);
        // And a null instance is a no on both, never a crash.
        let mut c = untouched();
        (API.wheel)(std::ptr::null_mut(), 3.0, r, 100.0, 100.0, &mut c);
        (API.click)(std::ptr::null_mut(), 1.0, 1.0, r, 100.0, 100.0, &mut c);
        assert_eq!(c.kind, ACTION_NONE);
        (API.destroy)(inst);
    }

    /// The key entry, driven through the table: a character lands in
    /// the prompt and is consumed; TAB is left to the host's focus
    /// chain; a null instance consumes nothing and never crashes.
    #[test]
    fn the_key_entry_answers_for_the_prompt_and_leaves_tab_to_the_host() {
        let inst = (API.create)();
        assert!(!inst.is_null());
        let mut a = untouched();
        assert_eq!((API.key)(inst, 'x' as u32, std::ptr::null(), 0, 0, &mut a), 1);
        assert_eq!(a.kind, ACTION_NONE, "consumed, and nothing asked of the host");
        let tab = keys::TAB;
        assert_eq!((API.key)(inst, 0, tab.as_ptr(), tab.len() as u32, 0, &mut a), 0);
        assert_eq!(
            (API.key)(std::ptr::null_mut(), 'x' as u32, std::ptr::null(), 0, 0, &mut a),
            0
        );
        (API.destroy)(inst);
    }

    /// The key channel translates what it can and refuses the rest —
    /// the same reading every field-carrying panel here gives it.
    #[test]
    fn the_key_channel_translates_what_it_can_and_refuses_the_rest() {
        assert_eq!(key_ev('a' as u32, None, 0).map(|e| e.key), Some(Key::Char('a')));
        assert_eq!(key_ev(0, Some("ENTER"), 0).map(|e| e.key), Some(Key::Enter));
        assert!(key_ev(0, Some("F13"), 0).is_none());
        assert!(key_ev(0x1b, None, 0).is_none());
        assert!(key_ev(0, None, 0).is_none());
        unsafe {
            assert_eq!(label_of(b"UP".as_ptr(), 2), Some("UP"));
            assert_eq!(label_of(b"UP".as_ptr(), 0), None);
            assert_eq!(label_of(std::ptr::null(), 4), None);
            assert_eq!(label_of([0xffu8, 0xfe].as_ptr(), 2), None);
        }
    }
}
