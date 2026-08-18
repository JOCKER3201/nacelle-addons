//! MEDIA LOOP panel — the first of the AI tools on the SEARCH AND AI
//! board: a path in, the daemon's `loop` tool out.
//!
//! Give it a VIDEO and the daemon makes a seamless loop of it; give it
//! a photo and the daemon makes a one-minute looped clip. The result is
//! always a NEW file beside the source — the daemon's contract, spelled
//! in `.gap-program/decyzja-nacelle-ai-daemon.md` — and the panel's
//! whole answer is that file's path.
//!
//! # What the panel is made of
//!
//! Four things, top to bottom, inside the content box the host's panel
//! container hands over:
//!
//! 1. a PATH FIELD — the toolkit's text input, drawn by [`field`]
//!    exactly as the search box is. Typing a path is the whole input
//!    story today; drag-and-drop is the daemon decision file's future,
//!    not this file's present.
//! 2. a START button — one click, one `{"cmd":"tool","tool":"loop"}`
//!    line to the daemon, through the shared [`nacelle_ai_client`].
//!    While the daemon asks for approval the row holds ALLOW and REFUSE
//!    instead, because that is the question on the table.
//! 3. the run's story — the daemon's `progress` lines as they arrive,
//!    newest at the top of the strip.
//! 4. the ANSWER — the new file's path on `done`, or the daemon's own
//!    words on `error`.
//!
//! # Offline is the empty state
//!
//! No daemon, no controls: the panel draws the theme's `[emptystate]`
//! message and nothing else, exactly as the old AI panel did — a field
//! that accepts a path nobody will ever read is the dead text field
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
use crate::model::{Model, Outcome, Phase};
use nacelle::focus::{Key, KeyEv, Mods};
use nacelle::runtime::{
    keys, ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_NONE,
    SIZING_REFERENCE,
};
use nacelle::theme::parse::State;
use nacelle::ui::Align;
use nacelle::view::paint::{self, RoleLook};
use nacelle::view::surface::{AbiSurface, Surface};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use nacelle_ai_client::{AiClient, Status, Tool};
use serde_json::json;
use std::ffi::c_void;

/// The name the host's title band shows, and the name the client's
/// `hello` announces — one word, so a daemon-side trace and a screen
/// read the same.
static TITLE: &[u8] = b"MEDIA LOOP";

/// The title band's right half while the daemon is away — a fact about
/// the panel, where the file browser puts its cwd.
static SAY_OFFLINE_BAND: &[u8] = b"offline";

/// The client name on the wire. Lowercase like the addon's file stem:
/// `ailoop.so`, `client":"ailoop"`.
const CLIENT: &str = "ailoop";

/// What the empty box invites. A path, not a search: the box is handed
/// to the daemon verbatim.
const PLACEHOLDER: &str = "path to a video or a photo";

/// The empty state, when there is no daemon to hand anything to. It
/// names the thing that is missing and promises no date, for the same
/// reason the old AI panel's message did: this file has no way of
/// knowing one.
const SAY_OFFLINE: &str =
    "Waiting for the nacelle-ai daemon. Media looping takes no input until that daemon answers.";

/// The captions, as content. The CASE they draw in is the button
/// role's `case` token's decision, not these strings'.
const CAP_START: &str = "start";
const CAP_ALLOW: &str = "allow";
const CAP_REFUSE: &str = "refuse";

/// The status words, one per phase — the small line over the story.
const SAY_WORKING: &str = "working\u{2026}";
const SAY_DONE_NO_FILE: &str = "done \u{2014} the daemon named no file";

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


/// One button: the `button` class's rung for its state, under the
/// `button.*` shape tokens and the role `button.role` binds — the same
/// names the CONTROL panel draws its two from, so the interface's
/// buttons are one object and not a family of lookalikes.
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
    // The case is the ROLE's, carried on the look this line already
    // holds. This widget kept its own copy of the transform — and its
    // own copy of "which key names it" — beside a `RoleLook` that could
    // not answer either; both are `paint::role_look`'s now.
    let cap = role.cased(caption);
    let ty = paint::center_line_y(sf, r.y, r.h, role.px, role.leading);
    sf.text(role.face, role.px, r.cx(), ty, &cap, ink.text, role.track, Align::Center);
}

/// The width a button asks for: its caption at the button role, plus
/// the padding the master declares for a BUTTON on either side. No
/// length of this file's own — and no borrowed one either: this used to
/// measure with `field.pad_x`, so a theme that gave its buttons more
/// breathing room than the box you type into got neither, and
/// `button.pad_x` sat in the master with nothing reading it.
fn button_w(sf: &mut impl Surface, caption: &str) -> f32 {
    let role = paint::bound_role(sf, "button.role", 1.0);
    // The case is the ROLE's, carried on the look this line already
    // holds. This widget kept its own copy of the transform — and its
    // own copy of "which key names it" — beside a `RoleLook` that could
    // not answer either; both are `paint::role_look`'s now.
    let cap = role.cased(caption);
    let pad = sf.px("button.pad_x").max(0.0);
    sf.measure(role.face, role.px, &cap, role.track) + 2.0 * pad
}

/// The message broken to fit `max_w`, greedily, at spaces — the old AI
/// panel's wrap, kept for the same message in the same place. A line
/// always takes its first word however narrow the box is, so a `max_w`
/// that is zero or nonsense gives one word per line rather than a loop
/// that does not end. The measurement is `FnMut` because measuring
/// through a [`Surface`] borrows the surface mutably.
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

// ------------------------------------------------------------- the widget

pub struct AiLoop {
    /// The model: the path, the run, the story.
    model: Model,
    /// The path box's between-frame state.
    field: FieldView,
    /// The daemon's client: one non-blocking socket, polled once per
    /// frame from the draw path, Offline until it answers.
    client: AiClient,
    /// What the last frame drew, for the input that arrives with no
    /// geometry of its own. Zeroed while the daemon is away, so an
    /// offline panel's clicks land on nothing — the controls are not
    /// there, and neither are their rectangles.
    field_r: Rect,
    start_r: Rect,
    allow_r: Rect,
    refuse_r: Rect,
    /// Whether the last poll saw the daemon, so the frame after a
    /// hang-up can tell the model its run died with the socket.
    was_connected: bool,
}

const NO_RECT: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

impl AiLoop {
    pub fn new() -> AiLoop {
        AiLoop::with_client(AiClient::new(CLIENT))
    }

    /// The same widget over an explicit client — the seam the tests
    /// build through, aimed at a socket path no daemon listens on.
    pub fn with_client(client: AiClient) -> AiLoop {
        AiLoop {
            model: Model::new(),
            field: FieldView::new(),
            client,
            field_r: NO_RECT,
            start_r: NO_RECT,
            allow_r: NO_RECT,
            refuse_r: NO_RECT,
            was_connected: false,
        }
    }

    /// Sends the run. `true` when the command went out — the model is
    /// told the id so every later event finds its request. `false` is
    /// Offline or nothing to send, and nothing is queued for a daemon
    /// that is not there: the client's own rule, kept here.
    fn start(&mut self) -> bool {
        if !self.model.can_start() {
            return false;
        }
        let args = json!({ "path": self.model.path() });
        match self.client.tool(Tool::Loop, &args) {
            Some(id) => {
                self.model.started(id);
                true
            }
            None => false,
        }
    }

    /// The user answered the approval question.
    fn answer(&mut self, allow: bool) {
        if let Some(id) = self.model.answered(allow) {
            self.client.approve(id, allow);
        }
    }

    /// One frame's worth of daemon: knock or read, then fold every
    /// event that arrived into the model. The draw path is the pump —
    /// the client counts polls, not seconds, so an occluded panel costs
    /// nothing and knocks at nothing.
    fn pump(&mut self) {
        self.client.poll();
        while let Some(ev) = self.client.take_event() {
            self.model.on_event(&ev);
        }
        let connected = self.client.status() == Status::Connected;
        if self.was_connected && !connected {
            self.model.connection_lost();
        }
        self.was_connected = connected;
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
        let message = role.cased(SAY_OFFLINE);
        let y_frac = sf.px("emptystate.y_frac");
        let lines = wrap(&message, r.w, |s: &str| {
            sf.measure(role.face, role.px, s, role.track)
        });
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

    /// The status strip: the phase's word, the main line, then the
    /// story so far, newest first — as many rows as fit, and not one
    /// drawn outside the box.
    fn strip(&mut self, sf: &mut impl Surface, r: Rect) {
        let label = paint::bound_role(sf, "list.label_role", 1.0);
        let status = paint::bound_role(sf, "list.status_role", 1.0);
        let row_h = sf.px("list.row_h").max(1.0);
        let gap = sf.px("list.gap").max(0.0);
        let pitch = row_h + gap;

        // The lines, in the order they are drawn: (text, main?). The
        // main line is the phase's answer; the rest is the log, newest
        // first, because the latest word is the one being waited on.
        let mut lines: Vec<(String, bool)> = Vec::new();
        match &self.model.phase {
            Phase::Idle => {}
            Phase::Running { .. } => {
                let latest = self.model.log.back().cloned();
                lines.push((latest.unwrap_or_else(|| SAY_WORKING.to_string()), true));
            }
            Phase::Waiting { desc, .. } => lines.push((desc.clone(), true)),
            Phase::Finished { path, .. } => match path {
                Some(p) => lines.push((p.clone(), true)),
                None => lines.push((SAY_DONE_NO_FILE.to_string(), true)),
            },
            Phase::Failed { msg, .. } => lines.push((msg.clone(), true)),
        }
        // What led up to it. The line already shown as the main one is
        // not repeated under itself.
        let shown = match &self.model.phase {
            Phase::Running { .. } => self.model.log.len().saturating_sub(1),
            _ => self.model.log.len(),
        };
        for line in self.model.log.iter().take(shown).rev() {
            lines.push((line.clone(), false));
        }

        let clipped = sf.clip(r);
        for (i, (text, main)) in lines.iter().enumerate() {
            let y = r.y + i as f32 * pitch;
            if y + row_h > r.bottom() {
                break;
            }
            let role: &RoleLook = if *main { &label } else { &status };
            if role.px <= 0.0 {
                continue;
            }
            let fit = paint::fit_end(sf, role.face, role.px, text, r.w, role.track);
            let ty = paint::center_line_y(sf, y, row_h, role.px, role.leading);
            sf.text(role.face, role.px, r.x, ty, &fit, role.color, role.track, Align::Left);
        }
        if clipped {
            sf.unclip();
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let mut sf = AbiSurface::new(api, ctx);
        self.pump();

        if self.client.status() == Status::Offline {
            // No daemon, no controls, no rectangles: a click on where
            // the button was must land on nothing.
            self.field_r = NO_RECT;
            self.start_r = NO_RECT;
            self.allow_r = NO_RECT;
            self.refuse_r = NO_RECT;
            self.offline(&mut sf, r);
            return;
        }

        let gap = sf.px("list.gap").max(0.0);
        let field_h = sf.px("field.h").max(0.0);

        // The path box. As in the search panel, there is no focus chain
        // across the ABI, so the panel answers the question the chain
        // would: the box owns the keyboard while it is on screen.
        let field_r = Rect::new(r.x, r.y, r.w, field_h.min(r.h));
        field::draw(&mut sf, field_r, &self.model.input, &mut self.field, PLACEHOLDER, true);
        self.field_r = field_r;

        // The button row: START, or the approval question's two.
        self.start_r = NO_RECT;
        self.allow_r = NO_RECT;
        self.refuse_r = NO_RECT;
        let row_y = field_r.bottom() + gap;
        let row_h = field_h.min((r.bottom() - row_y).max(0.0));
        let mut strip_top = row_y;
        if row_h > 0.0 {
            match self.model.phase {
                Phase::Waiting { .. } => {
                    let (mx, my) = sf.mouse();
                    let aw = button_w(&mut sf, CAP_ALLOW).min(r.w);
                    let rw = button_w(&mut sf, CAP_REFUSE).min((r.w - aw - gap).max(0.0));
                    let allow_r = Rect::new(r.x, row_y, aw, row_h);
                    let refuse_r = Rect::new(r.x + aw + gap, row_y, rw, row_h);
                    let state = |r: &Rect| {
                        if r.contains(mx, my) {
                            State::Hover
                        } else {
                            State::Idle
                        }
                    };
                    button(&mut sf, allow_r, CAP_ALLOW, state(&allow_r));
                    button(&mut sf, refuse_r, CAP_REFUSE, state(&refuse_r));
                    self.allow_r = allow_r;
                    self.refuse_r = refuse_r;
                    strip_top = row_y + row_h + gap;
                }
                Phase::Running { .. } => {
                    // Nothing to press: the run is the daemon's until it
                    // answers or asks. The row is given to the story.
                }
                _ => {
                    let (mx, my) = sf.mouse();
                    let w = button_w(&mut sf, CAP_START).min(r.w);
                    let start_r = Rect::new(r.x, row_y, w, row_h);
                    // A START with no path is DISABLED, and drawn as
                    // such: the theme's own word for "not now", instead
                    // of a live-looking button that swallows the click.
                    let state = if !self.model.can_start() {
                        State::Disabled
                    } else if start_r.contains(mx, my) {
                        State::Hover
                    } else {
                        State::Idle
                    };
                    button(&mut sf, start_r, CAP_START, state);
                    self.start_r = start_r;
                    strip_top = row_y + row_h + gap;
                }
            }
        }

        // The story, under whatever the row above left.
        let strip_r = Rect::new(r.x, strip_top, r.w, (r.bottom() - strip_top).max(0.0));
        if strip_r.h > 0.0 {
            self.strip(&mut sf, strip_r);
        }
    }

    /// A press. The rectangles are the last frame's, because a click
    /// arrives with no geometry of its own — the search panel's rule.
    pub fn click(&mut self, x: f32, y: f32) {
        if self.field_r.contains(x, y) {
            let at = self.field.hit(x);
            self.model
                .input
                .apply(nacelle::object::text_input::InputMsg::Point { at, extend: false });
            return;
        }
        if self.start_r.contains(x, y) {
            self.start();
            return;
        }
        if self.allow_r.contains(x, y) {
            self.answer(true);
            return;
        }
        if self.refuse_r.contains(x, y) {
            self.answer(false);
        }
    }

    /// A key delivered to this panel. Answers whether it was consumed.
    pub fn key(&mut self, ev: &KeyEv) -> bool {
        match self.model.key(ev) {
            Outcome::Start => self.start(),
            Outcome::Edited => true,
            Outcome::Ignored => false,
        }
    }
}

impl Default for AiLoop {
    fn default() -> Self {
        AiLoop::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(AiLoop::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut AiLoop) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut AiLoop> {
    unsafe { (instance as *mut AiLoop).as_mut() }
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

/// Nothing here scrolls yet: the story strip shows what fits and the
/// newest line is always at its top, so the wheel has nothing to reveal
/// that the strip is hiding. When the strip grows a real scroll it goes
/// through the toolkit's `view::scroll`, not arithmetic invented here.
extern "C" fn wheel_c(
    _: *mut c_void,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
    out: *mut ActionC,
) {
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
/// the story strip, not the size of anything in it.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    SIZING_REFERENCE
}

/// The header: the panel's name, and — while the daemon is away — the
/// one fact about the panel worth the right half of the band.
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
    if this.client.status() == Status::Offline {
        out.right = SAY_OFFLINE_BAND.as_ptr();
        out.right_len = SAY_OFFLINE_BAND.len() as u32;
    }
    (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
}

/// This widget takes no drags: there is no thumb to hold and no row to
/// tear off. Declining every Begin keeps a press on the ordinary click
/// path, which is where the buttons are.
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
/// host's. When the press phase carries something this panel wants, it
/// arrives here and gets a body.
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
/// `ailoop.so` from the addons directory. The name and the metadata are
/// the addon's own — the same string the file would be called and the
/// very bytes of `ailoop.meta` beside it — so a host never describes a
/// widget it merely links.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "ailoop",
    meta: include_str!("../ailoop.meta"),
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
        assert_eq!(def.name, "ailoop");
        assert_eq!(def.label, "MEDIA LOOP");
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
    /// it — the field view, the buttons, the strip and the empty state.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise: a widget that names a token the master does not
    /// declare degrades silently to nothing drawn, so a typo fails
    /// nowhere else — and therefore fails here.
    const TOKENS: &[&str] = &[
        // the path box — [field] and its component colours
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
        "button.pad_x",
        // the story strip
        "list.row_h",
        "list.gap",
        "list.label_role",
        "list.status_role",
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
        for class in ["field", "button"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::model::{done_path, SAY_ALLOWED, SAY_GONE, SAY_REFUSED};
    use nacelle_ai_client::Event;
    use serde_json::Value;

    fn running(id: u64) -> Model {
        let mut m = Model::new();
        m.input.set_value("/tmp/clip.mp4");
        m.started(id);
        m
    }

    /// The whole point of the daemon's events is what they do to the
    /// panel, and this is that mapping, event by event: progress and
    /// delta write the story, approval opens the question, done closes
    /// the run with the new file's path, error closes it with why.
    #[test]
    fn every_event_kind_lands_where_the_widget_shows_it() {
        let mut m = running(7);
        assert_eq!(m.phase, Phase::Running { id: 7 });

        m.on_event(&Event::Progress { id: 7, msg: "reading frames".into() });
        assert_eq!(m.log.back().map(String::as_str), Some("reading frames"));
        m.on_event(&Event::Delta { id: 7, text: "crossfade at 12s".into() });
        assert_eq!(m.log.back().map(String::as_str), Some("crossfade at 12s"));
        assert_eq!(m.phase, Phase::Running { id: 7 }, "the story does not move the phase");

        m.on_event(&Event::Approval { id: 7, desc: "run ffmpeg on clip.mp4".into() });
        assert_eq!(
            m.phase,
            Phase::Waiting { id: 7, desc: "run ffmpeg on clip.mp4".into() }
        );

        // The user allows; the answer goes out under the run's own id
        // and the run goes back to being the daemon's.
        assert_eq!(m.answered(true), Some(7));
        assert_eq!(m.phase, Phase::Running { id: 7 });
        assert_eq!(m.log.back().map(String::as_str), Some(SAY_ALLOWED));

        let body: Value =
            serde_json::from_str(r#"{"ev":"done","id":7,"out":"/tmp/clip-loop.mp4"}"#).unwrap();
        m.on_event(&Event::Done { id: 7, body });
        assert_eq!(
            m.phase,
            Phase::Finished { id: 7, path: Some("/tmp/clip-loop.mp4".into()) }
        );
    }

    #[test]
    fn an_error_closes_the_run_with_the_daemons_own_words() {
        let mut m = running(3);
        m.on_event(&Event::Error { id: 3, msg: "no such file".into() });
        assert_eq!(m.phase, Phase::Failed { id: 3, msg: "no such file".into() });
        // And a late progress line about the dead run changes nothing.
        m.on_event(&Event::Progress { id: 3, msg: "late".into() });
        assert_eq!(m.phase, Phase::Failed { id: 3, msg: "no such file".into() });
        assert!(m.log.is_empty());
    }

    /// Events about a request this panel did not send — a stale id, an
    /// echo, a v1 daemon reusing numbers — are stepped over: the state
    /// they would move belongs to a different conversation.
    #[test]
    fn an_event_about_somebody_elses_request_is_dropped() {
        let mut m = running(7);
        m.on_event(&Event::Progress { id: 8, msg: "not ours".into() });
        assert!(m.log.is_empty());
        m.on_event(&Event::Done { id: 8, body: serde_json::json!({}) });
        m.on_event(&Event::Error { id: 8, msg: "not ours".into() });
        m.on_event(&Event::Approval { id: 8, desc: "not ours".into() });
        assert_eq!(m.phase, Phase::Running { id: 7 });
        // The handshake is the client's business, not the run's.
        m.on_event(&Event::Hello { proto: 0, backends: vec!["local".into()] });
        assert_eq!(m.phase, Phase::Running { id: 7 });
    }

    /// A `done` that names no file is shown as exactly that: `None`,
    /// never a guessed path. The words a result may travel under are
    /// the three the daemon's side plausibly writes.
    #[test]
    fn the_result_path_is_read_from_the_done_body() {
        let path = |s: &str| done_path(&serde_json::from_str::<Value>(s).unwrap());
        assert_eq!(path(r#"{"ev":"done","id":1,"out":"/a/b-loop.mp4"}"#), Some("/a/b-loop.mp4".into()));
        assert_eq!(path(r#"{"ev":"done","id":1,"path":"/a/b.mp4"}"#), Some("/a/b.mp4".into()));
        assert_eq!(path(r#"{"ev":"done","id":1,"result":"/a/b.mp4"}"#), Some("/a/b.mp4".into()));
        assert_eq!(path(r#"{"ev":"done","id":1}"#), None);
        // A result that is not a string is not a path.
        assert_eq!(path(r#"{"ev":"done","id":1,"out":42}"#), None);
    }

    /// The daemon hanging up mid-run is the failure the panel must not
    /// render as eternal progress: the run died with the socket and the
    /// panel says so. With nothing in flight, nothing is lost.
    #[test]
    fn a_lost_connection_fails_the_run_in_flight_and_only_that() {
        let mut m = running(5);
        m.connection_lost();
        assert_eq!(m.phase, Phase::Failed { id: 5, msg: SAY_GONE.into() });

        let mut idle = Model::new();
        idle.connection_lost();
        assert_eq!(idle.phase, Phase::Idle);

        let mut done = running(6);
        done.on_event(&Event::Done { id: 6, body: serde_json::json!({"out":"/x"}) });
        done.connection_lost();
        assert_eq!(
            done.phase,
            Phase::Finished { id: 6, path: Some("/x".into()) },
            "a finished answer survives the daemon leaving"
        );
    }

    /// One request at a time, and a new run opens a new story: START is
    /// refusable while one runs, and starting clears the old log.
    #[test]
    fn one_request_at_a_time_and_a_fresh_log_per_run() {
        let mut m = running(1);
        assert!(!m.can_start(), "a running panel cannot start a second run");
        m.on_event(&Event::Approval { id: 1, desc: "?".into() });
        assert!(!m.can_start(), "an open question is still one run");
        assert_eq!(m.answered(false), Some(1));
        assert_eq!(m.log.back().map(String::as_str), Some(SAY_REFUSED));
        m.on_event(&Event::Error { id: 1, msg: "refused".into() });
        assert!(m.can_start(), "a closed run frees the button");
        m.started(2);
        assert!(m.log.is_empty(), "run 2 does not inherit run 1's story");
        // And answering with no question open answers nothing.
        assert_eq!(m.answered(true), None);
    }

    /// The path handed to the daemon is the box's value trimmed; a box
    /// of whitespace is an empty box, and an empty box cannot start.
    #[test]
    fn the_path_is_trimmed_and_an_empty_box_cannot_start() {
        let mut m = Model::new();
        assert!(!m.can_start());
        m.input.set_value("   ");
        assert!(!m.can_start());
        m.input.set_value("  /tmp/a clip.mp4 \n");
        assert_eq!(m.path(), "/tmp/a clip.mp4");
        assert!(m.can_start());
    }

    /// The story is a strip, not a terminal: the log keeps the last
    /// [`crate::model::LOG_KEEP`] lines and drops the oldest, and an
    /// empty line — a daemon clearing its throat — is not a line.
    #[test]
    fn the_log_is_bounded_and_skips_empty_lines() {
        let mut m = running(1);
        m.on_event(&Event::Progress { id: 1, msg: String::new() });
        assert!(m.log.is_empty());
        for i in 0..(crate::model::LOG_KEEP + 5) {
            m.on_event(&Event::Progress { id: 1, msg: format!("step {i}") });
        }
        assert_eq!(m.log.len(), crate::model::LOG_KEEP);
        assert_eq!(m.log.front().map(String::as_str), Some("step 5"), "oldest fell off");
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    fn press(m: &mut Model, ch: char, label: Option<&str>) -> Outcome {
        let ev = key_ev(ch as u32, label, 0).expect("a key this build knows");
        m.key(&ev)
    }

    #[test]
    fn typing_edits_the_path_and_enter_starts_it() {
        let mut m = Model::new();
        for c in "/tmp/a.mp4".chars() {
            assert_eq!(press(&mut m, c, None), Outcome::Edited);
        }
        assert_eq!(m.path(), "/tmp/a.mp4");
        assert_eq!(press(&mut m, '\0', Some("ENTER")), Outcome::Start);
    }

    #[test]
    fn enter_over_nothing_or_over_a_run_is_not_consumed() {
        let mut m = Model::new();
        assert_eq!(press(&mut m, '\0', Some("ENTER")), Outcome::Ignored);
        m.input.set_value("/tmp/a.mp4");
        m.started(1);
        assert_eq!(
            press(&mut m, '\0', Some("ENTER")),
            Outcome::Ignored,
            "one request at a time — Enter cannot start a second"
        );
    }

    #[test]
    fn escape_empties_the_box_once_and_bubbles_after() {
        let mut m = Model::new();
        m.input.set_value("/tmp/a.mp4");
        assert_eq!(press(&mut m, '\0', Some("ESC")), Outcome::Edited);
        assert_eq!(m.input.value(), "");
        // The second Escape belongs to whatever put the panel on
        // screen.
        assert_eq!(press(&mut m, '\0', Some("ESC")), Outcome::Ignored);
    }

    #[test]
    fn the_key_channel_translates_what_it_can_and_refuses_the_rest() {
        assert_eq!(key_ev('a' as u32, None, 0).map(|e| e.key), Some(Key::Char('a')));
        assert_eq!(key_ev(0, Some("ENTER"), 0).map(|e| e.key), Some(Key::Enter));
        // A name this build does not know is not a key, and neither is
        // a control character.
        assert!(key_ev(0, Some("F13"), 0).is_none());
        assert!(key_ev(0x1b, None, 0).is_none());
        assert!(key_ev(0, None, 0).is_none());
        // The label pointer, as the boundary really passes one.
        unsafe {
            assert_eq!(label_of(b"UP".as_ptr(), 2), Some("UP"));
            assert_eq!(label_of(b"UP".as_ptr(), 0), None);
            assert_eq!(label_of(std::ptr::null(), 4), None);
            assert_eq!(label_of([0xffu8, 0xfe].as_ptr(), 2), None);
        }
    }
}

#[cfg(test)]
mod button_tests {
    use super::*;
    use nacelle::theme::Color;

    /// A probe with an opinion about exactly two keys, so the width a
    /// button asks for says WHICH of them it measured itself with.
    /// Everything else is inert: a caption's width is arithmetic, and
    /// arithmetic needs no window.
    struct Pads {
        button: f32,
        field: f32,
    }
    impl Surface for Pads {
        fn ring_fill(&mut self, _: Rect, _: nacelle::draw::CornerStyle, _: f32, _: Color) {}
        fn ring(&mut self, _: Rect, _: nacelle::draw::CornerStyle, _: f32, _: f32, _: Color) {}
        fn rect(&mut self, _: Rect, _: Color) {}
        fn rect_outline(&mut self, _: Rect, _: f32, _: Color) {}
        fn line(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: Color) {}
        fn polyline(&mut self, _: &[[f32; 2]], _: f32, _: Color, _: bool) {}
        #[allow(clippy::too_many_arguments)]
        fn text(&mut self, _: u8, _: f32, _: f32, _: f32, _: &str, _: Color, _: f32, _: Align) {}
        fn measure(&mut self, _: u8, _: f32, s: &str, _: f32) -> f32 {
            s.chars().count() as f32 * 10.0
        }
        fn clip(&mut self, _: Rect) -> bool {
            false
        }
        fn unclip(&mut self) {}
        fn has_token(&mut self, _: &str) -> bool {
            false
        }
        fn px(&mut self, name: &str) -> f32 {
            match name {
                "button.pad_x" => self.button,
                "field.pad_x" => self.field,
                _ => 0.0,
            }
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
        /// Nothing, like every other token this surface is asked for:
        /// it measures runs and draws none, so no trim is reached.
        fn theme_text(&mut self, _: &str) -> String {
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

    /// A button is as wide as the master says a BUTTON is padded.
    ///
    /// It used to measure itself with `field.pad_x` — the box you type
    /// into — so `button.pad_x` sat in the master with no reader and a
    /// theme that padded its buttons got nothing. The two keys differ in
    /// the master today (`@space.5` against `@space.4`), so this is not
    /// a distinction without a difference.
    #[test]
    fn a_button_is_as_wide_as_the_button_padding_makes_it() {
        const CAP: &str = "START";
        let text = CAP.chars().count() as f32 * 10.0;
        assert_eq!(
            button_w(&mut Pads { button: 7.0, field: 100.0 }, CAP),
            text + 14.0,
            "a button's width is its caption plus `button.pad_x` on either side"
        );
        // Moving the FIELD's padding must not move a button by a pixel.
        assert_eq!(
            button_w(&mut Pads { button: 7.0, field: 999.0 }, CAP),
            text + 14.0,
            "the button followed `field.pad_x`, which belongs to the path box"
        );
        // And moving the button's own must.
        assert_eq!(
            button_w(&mut Pads { button: 20.0, field: 100.0 }, CAP),
            text + 40.0,
            "`button.pad_x` moved and the button did not"
        );
    }
}

#[cfg(test)]
mod offline_tests {
    use super::*;
    use nacelle_ai_client::AiClient;
    use std::path::PathBuf;

    /// A widget whose daemon does not exist: aimed at a socket path
    /// under a directory that is not there, so connect fails without
    /// touching anything real.
    fn stranded() -> AiLoop {
        AiLoop::with_client(AiClient::at(CLIENT, PathBuf::from("/nonexistent-ailoop/ai.sock")))
    }

    /// Offline, the controls are NOT THERE — not merely unresponsive.
    /// The last frame's rectangles are zeroed before the empty state is
    /// drawn, so this is testable without a surface: a click anywhere
    /// lands on nothing and changes nothing.
    #[test]
    fn offline_takes_no_click_and_starts_nothing() {
        let mut w = stranded();
        w.client.poll();
        assert_eq!(w.client.status(), Status::Offline);
        w.model.input.set_value("/tmp/a.mp4");
        // The rects a widget starts with are the no-rects; a click that
        // would have hit START on a live panel hits nothing here.
        w.click(1.0, 1.0);
        assert_eq!(w.model.phase, Phase::Idle);
        // And the model's own start path refuses too: the client is
        // Offline, so the command goes nowhere and the phase stays put.
        assert!(!w.start());
        assert_eq!(w.model.phase, Phase::Idle);
    }

    /// Enter in the field while Offline: the field consumes its own
    /// keys (typing must not leak to the host because a daemon died),
    /// but Enter starts nothing — `start` answers the client's `None`
    /// by leaving the phase alone.
    #[test]
    fn enter_while_offline_starts_nothing() {
        let mut w = stranded();
        w.client.poll();
        for c in "/tmp/a.mp4".chars() {
            let ev = key_ev(c as u32, None, 0).unwrap();
            assert!(w.key(&ev));
        }
        let enter = key_ev(0, Some("ENTER"), 0).unwrap();
        assert!(!w.key(&enter), "an Enter that started nothing was not consumed");
        assert_eq!(w.model.phase, Phase::Idle);
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

    /// The inputs this widget does not use are INERT, and pinned so: the
    /// wheel scrolls nothing, the drag captures nothing, the pointer
    /// asks for no cursor, the press rung writes nothing. Each is a
    /// decision written above its entry, and a change that gives one a
    /// body has to come past this test rather than around it.
    #[test]
    fn the_inputs_this_widget_does_not_use_are_inert() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);

        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        // The wheel: answered, and answered with "nothing to do".
        let mut a = untouched();
        (API.wheel)(std::ptr::null_mut(), 3.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_NONE);

        // The drag: every phase declined, no capture ever asked for.
        let mut b = untouched();
        for phase in [DRAG_BEGIN, DRAG_MOVE, DRAG_END] {
            (API.drag)(std::ptr::null_mut(), phase, 1.0, 1.0, r, 100.0, 100.0, &mut b);
        }
        assert_eq!(b.kind, u32::MAX, "a drag entry that does nothing writes nothing");

        // The pointer: no hand cursor anywhere.
        assert_eq!((API.pointer)(std::ptr::null_mut(), 1.0, 1.0, r, 100.0, 100.0), 0);

        // The press rung: filled, declared, and deliberately empty.
        let mut c = untouched();
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(std::ptr::null_mut(), phase, 1.0, 1.0, r, 100.0, 100.0, &mut c);
        }
        assert_eq!(c.kind, u32::MAX, "a press entry that does nothing writes nothing");

        // The grid: this widget has no cells to report.
        let (mut cols, mut rows) = (u32::MAX, u32::MAX);
        (API.grid)(std::ptr::null_mut(), &mut cols, &mut rows);
        assert_eq!((cols, rows), (u32::MAX, u32::MAX));
    }

    /// The key entry, driven through the table: a character lands in
    /// the field and is consumed; TAB is left to the host's focus
    /// chain; a null instance consumes nothing and never crashes.
    #[test]
    fn the_key_entry_answers_for_the_field_and_leaves_tab_to_the_host() {
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
}
