//! AI SORT panel — file sorting through the nacelle-ai daemon.
//!
//! One of the four AI tools on the SEARCH AND AI board, and the first
//! rule of all four is the same: the panel is a CLIENT. It runs nothing
//! itself, walks no directory, moves no file — it hands a path to the
//! daemon over `nacelle-ai-client`'s socket and renders what comes
//! back. The daemon's confidentiality line (redact, supervise,
//! approvals) is on the other side of that socket, which is exactly
//! where it has to be for this widget to be unable to go around it.
//!
//! # What the panel is
//!
//! One path box, one SORT button, and a message area under them:
//!
//! * the BOX is the toolkit's `object::text_input` model with the
//!   plugin-side field view ([`field`], the search plugin's acknowledged
//!   copy — see that file's header);
//! * the BUTTON sends `{"cmd":"tool","tool":"sort","args":{"path":...}}`
//!   through [`AiClient::tool`]; Enter in the box is the same press;
//! * the MESSAGE AREA is where the daemon's answer lands — progress,
//!   the result, or an error, which today is `"not built yet"`, and the
//!   panel shows that sentence rather than pretending otherwise.
//!
//! # Offline is a state, not an error
//!
//! No daemon → [`Status::Offline`] → the panel draws the theme's own
//! empty state saying so, takes no input for a queue nobody is at the
//! other end of, and keeps knocking through [`AiClient::poll`] — once
//! per drawn frame, which is the correct amount of knocking for a panel
//! nobody can see (see the client crate's head).
//!
//! Every colour, length, duration and word of the LOOK comes from the
//! theme: `[field]` and `component.field.*` for the box, `[button]` and
//! class `button` for the button, `[emptystate]` for every message.
//! A missing token degrades through the raw answers the ABI itself
//! gives, never through a number that used to be the design.

mod field;

use crate::field::FieldView;
use nacelle::focus::{Key, KeyEv, Mods};
use nacelle::object::text_input::{key_msg, InputEdited, InputModel, InputMsg};
use nacelle::runtime::{
    keys, ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_NONE,
};
use nacelle::theme::parse::State;
use nacelle::ui::Align;
use nacelle::view::paint;
use nacelle::view::surface::{AbiSurface, Surface};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use nacelle_ai_client::{AiClient, Event, Status, Tool};
use serde_json::Value;
use std::ffi::c_void;

/// The name the host's title band shows.
static TITLE: &[u8] = b"AI SORT";

/// The name `hello` announces, so a daemon-side trace says which panel
/// asked.
const CLIENT: &str = "aisort";

/// What the empty box invites. A path, not a query: this panel sorts a
/// DIRECTORY, and the placeholder says which one thing it wants.
const PLACEHOLDER: &str = "directory to sort";

/// The button's cap, recased by `type.<button.role>.case` exactly as
/// every other cap in the interface is — the WORD is the code's, the
/// look is the theme's.
const LABEL_SORT: &str = "sort";

/// The three sentences this panel says for itself. Words, not look —
/// the `[emptystate]` role decides everything about how they draw.
/// Everything else the message area shows is the DAEMON's sentence,
/// shown as sent: `"not built yet"` today, progress and results later.
const SAY_OFFLINE: &str =
    "Waiting for the nacelle-ai daemon. This panel takes no input until that daemon answers.";
const SAY_START: &str = "name a directory and press SORT";
const SAY_WORKING: &str = "asked the daemon\u{2026}";

/// The longest path the box will hold, in characters. The cap is load
/// bearing rather than tidy, for the search box's reason: the field
/// view records a caret stop per character every frame, each measured
/// from the start of the value, so drawing cost grows with the SQUARE
/// of the length. A path has no business being longer either way.
const MAX_PATH: usize = 1024;

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

// ------------------------------------------------------------------ job

/// What the panel is in the middle of. One job at a time, on purpose:
/// a second SORT while one runs would be two answers racing for one
/// message area, and nothing about sorting a directory wants a queue.
#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    /// Nothing asked. The message area shows the invitation.
    Idle,
    /// Request `id` is with the daemon. `note` is its latest progress
    /// sentence, shown until the next one replaces it.
    Running { id: u64, note: Option<String> },
    /// The daemon's last word on a finished job — a result's text or an
    /// error's message (today: `"not built yet"`). Shown until the next
    /// request replaces it.
    Said(String),
}

/// The sentence a `done` event carries, read from the fields a result
/// plausibly holds. The spec spells `done` as `{"ev":"done","id":N,...}`
/// and pins the tail down on the daemon's side only, so this reads what
/// it knows and falls back to the one word that is always true.
pub fn done_text(body: &Value) -> String {
    for key in ["msg", "text", "out"] {
        if let Some(s) = body.get(key).and_then(Value::as_str) {
            return s.to_owned();
        }
    }
    "done".to_owned()
}

/// One protocol event, absorbed into the job state. Answers whether the
/// state changed — the whole mapping from wire to widget, free of the
/// widget so a test can drive it without a socket or a window.
///
/// Only events carrying the RUNNING job's id move anything: an event
/// for a request this panel never made (another client's echo, a v1
/// novelty, a stale answer from before a reconnect) is stepped over
/// with the same forbearance the parser shows unknown lines. `hello`
/// carries no id and asks nothing of a sort panel.
pub fn absorb(job: &mut JobState, ev: &Event) -> bool {
    let JobState::Running { id, note } = job else { return false };
    let id = *id;
    match ev {
        Event::Progress { id: e, msg } if *e == id => {
            *note = Some(msg.clone());
            true
        }
        // A streamed answer grows; a progress note replaces. Both are
        // "the latest thing worth reading" by their own kind's rule.
        Event::Delta { id: e, text } if *e == id => {
            match note {
                Some(n) => n.push_str(text),
                None => *note = Some(text.clone()),
            }
            true
        }
        Event::Error { id: e, msg } if *e == id => {
            *job = JobState::Said(msg.clone());
            true
        }
        Event::Done { id: e, body } if *e == id => {
            *job = JobState::Said(done_text(body));
            true
        }
        _ => false,
    }
}

// ----------------------------------------------------------------- keys

/// The key event a boundary call means, if any — the search panel's
/// reading, verbatim: [`keys::from_name`] is the ABI's OWN spelling of
/// the neutral key set, unknown modifier bits are dropped rather than
/// kept, and a control character is no key at all.
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
/// that stay put for the call. That is what both key entries promise,
/// and it is why this is `unsafe` rather than a tidy helper: a safe
/// function that dereferences a pointer somebody else chose is a safe
/// function that can be made to read anything.
unsafe fn label_of<'a>(label: *const u8, label_len: u32) -> Option<&'a str> {
    if label.is_null() || label_len == 0 {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(label, label_len as usize)).ok()
}

// ----------------------------------------------------------- the widget

pub struct AiSort {
    /// The daemon's socket, polled once per drawn frame.
    client: AiClient,
    /// The path box's model — the toolkit's, not this crate's.
    input: InputModel,
    /// The path box's between-frame view state.
    field: FieldView,
    /// What the panel is in the middle of.
    job: JobState,
    /// What the last draw settled on, for input that arrives with no
    /// geometry of its own. Zeroed while Offline, so a click on a
    /// remembered rectangle cannot press a button that is not there.
    field_r: Rect,
    button_r: Rect,
}

impl AiSort {
    pub fn new() -> AiSort {
        AiSort::with_client(AiClient::new(CLIENT))
    }

    /// The same panel over an explicit client. This is the seam the
    /// tests build through — a client aimed where no daemon listens is
    /// a panel that is honestly Offline.
    pub fn with_client(client: AiClient) -> AiSort {
        AiSort {
            client,
            input: InputModel::new().with_max_len(MAX_PATH),
            field: FieldView::new(),
            job: JobState::Idle,
            field_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            button_r: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// The SORT press, wherever it came from — the button or Enter in
    /// the box. Answers whether anything was actually asked: an empty
    /// path is nothing to sort, and Offline queues nothing (the client's
    /// own rule — `tool` answers `None` and the command went NOWHERE).
    pub fn submit(&mut self) -> bool {
        let path = self.input.value().trim().to_owned();
        if path.is_empty() {
            return false;
        }
        match self.client.tool(Tool::Sort, &serde_json::json!({ "path": path })) {
            Some(id) => {
                self.job = JobState::Running { id, note: None };
                true
            }
            None => false,
        }
    }

    /// A key delivered to this panel. Answers whether it was CONSUMED —
    /// what stops the host from also spending it on focus navigation or
    /// a shortcut.
    pub fn key(&mut self, ev: &KeyEv) -> bool {
        let Some(msg) = key_msg(ev) else { return false };
        match self.input.apply(msg) {
            InputEdited::Edited | InputEdited::Moved => true,
            // Enter is the button. An Enter that asked nothing — empty
            // box, or no daemon to ask — is not a key this panel spent.
            InputEdited::Submit => self.submit(),
            InputEdited::Cancel => {
                // Escape empties the box; on an already empty box it
                // belongs to whatever put the panel on screen.
                if self.input.value().is_empty() {
                    return false;
                }
                self.input.set_value("");
                true
            }
            // The clipboard does not cross the plugin boundary, and a
            // rejected or empty edit changed nothing: none of these are
            // keys this panel used.
            InputEdited::CopyRequest { .. } | InputEdited::PasteRequest => false,
            InputEdited::Rejected | InputEdited::None => false,
        }
    }

    /// A press, against the last draw's rectangles.
    pub fn click(&mut self, x: f32, y: f32) {
        if self.button_r.contains(x, y) {
            self.submit();
        } else if self.field_r.contains(x, y) {
            // The caret goes where the pointer is, from the positions
            // the last draw recorded — a click arrives with no way to
            // measure text.
            let at = self.field.hit(x);
            let _ = self.input.apply(InputMsg::Point { at, extend: false });
        }
    }

    /// One frame's worth of socket: knock/flush/read, then fold every
    /// arrived event into the job.
    fn pump(&mut self) {
        self.client.poll();
        while let Some(ev) = self.client.take_event() {
            absorb(&mut self.job, &ev);
        }
    }

    /// A message in the theme's own empty state: `emptystate.role` says
    /// the type, `emptystate.y_frac` says where in the box the block
    /// centres, and a theme that declares neither draws nothing — the
    /// same silence every panel here degrades to.
    fn say(sf: &mut impl Surface, r: Rect, text: &str) {
        let look = paint::bound_role(sf, "emptystate.role", 1.0);
        if look.px <= 0.0 || look.color.a <= 0.0 {
            return;
        }
        let y_frac = sf.px("emptystate.y_frac");
        let pitch = look.px * look.leading.max(1.0);
        let lines = paint::wrap(sf, look.face, look.px, text, r.w, look.track);
        let block = lines.len() as f32 * pitch;
        let top = (r.y + r.h * y_frac - block / 2.0).max(r.y);
        for (i, line) in lines.iter().enumerate() {
            let y = top + i as f32 * pitch;
            // A line whose box does not fit whole is not emitted: the
            // squeezed panel's neighbours are somebody else's pixels.
            if y + pitch > r.bottom() {
                break;
            }
            sf.text(look.face, look.px, r.cx(), y, line, look.color, look.track, Align::Center);
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let mut sf = AbiSurface::new(api, ctx);
        self.pump();

        if self.client.status() == Status::Offline {
            // A job in flight died with the daemon — its id means
            // nothing to whatever answers the next `hello` — and the
            // controls leave the screen, so their rectangles leave the
            // click path with them.
            self.job = JobState::Idle;
            self.field_r = Rect::new(0.0, 0.0, 0.0, 0.0);
            self.button_r = Rect::new(0.0, 0.0, 0.0, 0.0);
            Self::say(&mut sf, r, SAY_OFFLINE);
            return;
        }

        // ---- the row: path box, then the button ----------------------
        let field_h = sf.px("field.h").max(0.0).min(r.h);
        let gap = sf.px("field.pad_x").max(0.0);

        // The button is sized by its own cap, in its own role and case,
        // floored at `button.min_w` — and never more than half the
        // panel, because the box is what this panel IS.
        // The case is the ROLE's, carried on the look. This widget spelled
        // `type.{word}.case` from the binding by hand and then folded the
        // transform itself, which is the fifth copy of one rule the
        // toolkit now states once (`ui::recase`).
        let role = paint::bound_role(&mut sf, "button.role", 1.0);
        let cap = role.cased(LABEL_SORT);
        let cap_w = if role.px > 0.0 {
            sf.measure(role.face, role.px, &cap, role.track)
        } else {
            0.0
        };
        let bw = (cap_w + 2.0 * sf.px("button.pad_x").max(0.0))
            .max(sf.px("button.min_w"))
            .min((r.w / 2.0).max(0.0));
        let button_r = Rect::new(r.x + r.w - bw, r.y, bw, field_h);
        let field_r = Rect::new(r.x, r.y, (r.w - bw - gap).max(1.0), field_h);
        self.field_r = field_r;
        self.button_r = button_r;

        // There is no focus chain across this boundary, so the panel
        // answers the question the chain would: the one box owns the
        // keyboard for as long as it is on screen.
        field::draw(&mut sf, field_r, &self.input, &mut self.field, PLACEHOLDER, true);

        // The button: class `button`'s ladder over `[button]`'s shape.
        let (mx, my) = sf.mouse();
        let corner = paint::corner_radius(&mut sf, "button.corner", button_r, 1.0);
        let cut = paint::corner_style(&mut sf, "button.corner_style");
        let state = if button_r.contains(mx, my) { State::Hover } else { State::Idle };
        let ink = sf.class_state("button", state);
        sf.ring_fill(button_r, cut, corner, ink.fill);
        if ink.edge_width > 0.0 {
            sf.ring(button_r, cut, corner, ink.edge_width, ink.edge);
        }
        if role.px > 0.0 {
            let ty = paint::center_line_y(&mut sf, button_r.y, button_r.h, role.px, role.leading);
            // The ladder's own cap ink where the class states one; the
            // role's where it does not.
            let c = if ink.text.a > 0.0 { ink.text } else { role.color };
            sf.text(role.face, role.px, button_r.cx(), ty, &cap, c, role.track, Align::Center);
        }

        // ---- the message area ----------------------------------------
        let top = r.y + field_h + gap;
        let msg_r = Rect::new(r.x, top, r.w, (r.bottom() - top).max(0.0));
        let text = match &self.job {
            JobState::Idle => SAY_START.to_owned(),
            JobState::Running { note, .. } => {
                note.clone().unwrap_or_else(|| SAY_WORKING.to_owned())
            }
            JobState::Said(t) => t.clone(),
        };
        Self::say(&mut sf, msg_r, &text);
    }
}

impl Default for AiSort {
    fn default() -> Self {
        AiSort::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(AiSort::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut AiSort) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut AiSort> {
    unsafe { (instance as *mut AiSort).as_mut() }
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
        // Whatever was asked is already on its way to the daemon; the
        // host has nothing to do about it.
        out.kind = ACTION_NONE;
    }
}

/// One box and one button do not scroll. The wheel is not this panel's,
/// and saying so keeps it the board's.
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
/// asked of the HOST's table, where `channel_read` was appended by the
/// same change in the same version, exactly as the search panel asks.
fn routes_keys(api: &HostApi) -> bool {
    api.has_channel()
}

/// The BROADCAST, listened to only where nothing else is delivered: on
/// a host that routes keys, acting on both entries would type every
/// character twice. What is left here is the older host's path — a
/// path box with no keyboard at all would be worse than one without
/// modifiers.
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

/// The key delivered to the widget that OWNS the keyboard, modifiers
/// and all. Nonzero says the panel used the key, so the host must not
/// also spend it; `out` stays [`ACTION_NONE`] on every path, because
/// what Enter asks for goes to the daemon and not to the application.
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
    // Safety: the entry's contract — a null pointer or `label_len`
    // readable bytes that outlive the call.
    let label = unsafe { label_of(label, label_len) };
    let Some(ev) = key_ev(ch, label, mods) else {
        // A key this build cannot name and cannot spell is not one the
        // panel used; leaving it to the host is the honest answer.
        return 0;
    };
    this.key(&ev) as u32
}

/// Sized against the reference box on both axes: what grows here is a
/// message, whose height is the message's — nothing in this panel grows
/// in rows.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_REFERENCE
}

/// The header, as chrome: the panel's name, and no right-hand half yet.
/// The right half is where a panel says something ABOUT itself; when
/// the daemon can be asked, the connection state belongs there — until
/// then a status line would be the title band promising what the body
/// cannot.
extern "C" fn chrome_c(
    _: *mut c_void,
    _ctx: *mut c_void,
    _host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    let Some(out) = (unsafe { out.as_mut() }) else { return 0 };
    out.title = TITLE.as_ptr();
    out.title_len = TITLE.len() as u32;
    (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
}

/// This widget takes no drags: declining every Begin keeps a press on
/// the ordinary click path, which is where the button and the caret
/// live.
#[allow(clippy::too_many_arguments)]
extern "C" fn drag_c(
    _: *mut c_void,
    _: u32,
    _: f32,
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

/// The hand appears over the one control this panel has — the SORT
/// button, at the rectangle the last draw put it. While Offline that
/// rectangle is zeroed, so the hand cannot point at a button that is
/// not on screen.
extern "C" fn pointer_c(
    instance: *mut c_void,
    x: f32,
    y: f32,
    _r: RectC,
    _win_w: f32,
    _win_h: f32,
) -> u32 {
    let Some(this) = state(instance) else { return 0 };
    u32::from(this.button_r.contains(x, y))
}

/// Filled, and does nothing on purpose. The button is pressed through
/// the ordinary click path and wears no press rung of its own — its
/// ladder states are idle and hover, which the frame's own mouse
/// position already says. A press animation is the theme's to grow, not
/// this entry's to fake.
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
/// `aisort.so` from the addons directory. The name and the metadata are
/// the addon's own — the same string the file would be called and the
/// very bytes of `aisort.meta` beside it — so a host never describes a
/// widget it merely links.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "aisort",
    meta: include_str!("../aisort.meta"),
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
    /// declaration the host will read is the one this crate carries: the
    /// linked-in constant `include_str!`s the very file the installer
    /// copies, so the two cannot say different things.
    #[test]
    fn the_widget_registers_on_the_search_and_ai_board() {
        let def = registry::def_from_meta(WIDGET.name.to_string(), WIDGET.meta);
        assert_eq!(def.name, "aisort");
        assert_eq!(def.label, "AI SORT");
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
    /// Every token this crate names by a name of its own, spelled
    /// exactly as the code spells it — the whole of `field.rs` and the
    /// draw path above. The type roles themselves are NOT listed: they
    /// are reached through `view::paint`, which the toolkit's own tests
    /// cover, and the binding tokens that name them are.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise: a widget that names a token the master does not
    /// declare degrades silently, so a typo fails loudly nowhere else —
    /// which is why it fails here.
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
        // the button
        "button.role",
        "button.pad_x",
        "button.min_w",
        "button.corner",
        "button.corner_style",
        // and every message this panel shows
        "emptystate.role",
        "emptystate.y_frac",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
        // The classes the two controls rest on. A class the matrix does
        // not know answers the raw rung, and the panel would look
        // undesigned for a reason no reader could see.
        for class in ["field", "button"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;

    fn running(id: u64) -> JobState {
        JobState::Running { id, note: None }
    }

    /// The one answer the daemon gives today, mapped end to end: the
    /// `error` event's sentence becomes the panel's, verbatim. The
    /// widget neither rewrites it nor apologises for it — the daemon
    /// said "not built yet", and that is what the reader is told.
    #[test]
    fn not_built_yet_is_shown_as_the_daemon_said_it() {
        let mut job = running(7);
        let ev = Event::Error { id: 7, msg: "not built yet".into() };
        assert!(absorb(&mut job, &ev));
        assert_eq!(job, JobState::Said("not built yet".into()));
    }

    /// Progress replaces the note; a streamed delta grows it. Each kind
    /// keeps its own rule, and the latest thing worth reading is what
    /// the message area shows.
    #[test]
    fn progress_and_deltas_write_the_running_note() {
        let mut job = running(3);
        assert!(absorb(&mut job, &Event::Progress { id: 3, msg: "reading".into() }));
        assert_eq!(job, JobState::Running { id: 3, note: Some("reading".into()) });
        assert!(absorb(&mut job, &Event::Progress { id: 3, msg: "grouping".into() }));
        assert_eq!(job, JobState::Running { id: 3, note: Some("grouping".into()) });

        let mut job = running(4);
        assert!(absorb(&mut job, &Event::Delta { id: 4, text: "sorted 3 ".into() }));
        assert!(absorb(&mut job, &Event::Delta { id: 4, text: "of 9".into() }));
        assert_eq!(job, JobState::Running { id: 4, note: Some("sorted 3 of 9".into()) });
    }

    /// `done` ends the job with the sentence its body carries, read
    /// from the fields a result plausibly holds — and with the one word
    /// that is always true when it carries none.
    #[test]
    fn done_ends_the_job_with_its_own_sentence() {
        let mut job = running(9);
        let body: Value =
            serde_json::from_str(r#"{"ev":"done","id":9,"msg":"9 files sorted"}"#).unwrap();
        assert!(absorb(&mut job, &Event::Done { id: 9, body }));
        assert_eq!(job, JobState::Said("9 files sorted".into()));

        let mut job = running(2);
        let bare: Value = serde_json::from_str(r#"{"ev":"done","id":2}"#).unwrap();
        assert!(absorb(&mut job, &Event::Done { id: 2, body: bare }));
        assert_eq!(job, JobState::Said("done".into()));
    }

    /// Only the RUNNING job's id moves anything. Another request's
    /// events, and the id-less `hello`, are stepped over — the same
    /// forbearance the parser shows unknown lines.
    #[test]
    fn a_foreign_id_and_a_hello_change_nothing() {
        let mut job = running(7);
        for ev in [
            Event::Error { id: 8, msg: "somebody else's".into() },
            Event::Progress { id: 8, msg: "somebody else's".into() },
            Event::Delta { id: 8, text: "somebody else's".into() },
            Event::Done { id: 8, body: Value::Null },
            Event::Approval { id: 8, desc: "somebody else's".into() },
            Event::Hello { proto: 0, backends: vec!["local".into()] },
        ] {
            assert!(!absorb(&mut job, &ev), "{ev:?} is not job 7's");
            assert_eq!(job, running(7));
        }
    }

    /// A panel with no job in flight absorbs nothing: a stale answer
    /// arriving after `done` must not resurrect a message the user has
    /// already read past.
    #[test]
    fn idle_and_finished_absorb_nothing() {
        for mut job in [JobState::Idle, JobState::Said("done".into())] {
            let before = job.clone();
            assert!(!absorb(&mut job, &Event::Error { id: 1, msg: "late".into() }));
            assert!(!absorb(&mut job, &Event::Progress { id: 1, msg: "late".into() }));
            assert_eq!(job, before);
        }
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use std::path::PathBuf;

    /// A panel whose client knocks where no daemon listens: the scratch
    /// dir does not even exist, so every poll fails without touching
    /// anything real.
    fn offline() -> AiSort {
        AiSort::with_client(AiClient::at(CLIENT, PathBuf::from("/nonexistent-nacelle-test/ai.sock")))
    }

    fn press(s: &mut AiSort, ch: char, label: Option<&str>) -> bool {
        let ev = key_ev(ch as u32, label, 0).expect("a key this build knows");
        s.key(&ev)
    }

    /// Typing edits the path — the toolkit's field model, driven over
    /// this panel's own key path — and Escape empties it once, leaving
    /// the second Escape to whatever put the panel on screen.
    #[test]
    fn typing_edits_the_path_and_escape_clears_it() {
        let mut s = offline();
        for c in "/tmp/in".chars() {
            assert!(press(&mut s, c, None), "a character is the panel's");
        }
        assert_eq!(s.input.value(), "/tmp/in");
        assert!(press(&mut s, '\0', Some("ESC")));
        assert_eq!(s.input.value(), "");
        assert!(!press(&mut s, '\0', Some("ESC")), "an empty box owns no Escape");
    }

    /// Offline is a state a widget renders, not a queue it fills: Enter
    /// over a perfectly good path asks nothing, consumes nothing, and
    /// leaves the job Idle — the client's own no-spooling rule, visible
    /// from the widget's side.
    #[test]
    fn offline_takes_no_job() {
        let mut s = offline();
        s.pump();
        assert_eq!(s.client.status(), Status::Offline);
        for c in "/tmp/sort-me".chars() {
            press(&mut s, c, None);
        }
        assert!(!press(&mut s, '\0', Some("ENTER")), "Enter that asked nothing is not spent");
        assert!(!s.submit());
        assert_eq!(s.job, JobState::Idle);
    }

    /// Enter over an empty box is nobody's key: there is nothing to
    /// sort, so nothing is asked and the key is left to the host.
    #[test]
    fn enter_with_an_empty_path_asks_nothing() {
        let mut s = offline();
        assert!(!press(&mut s, '\0', Some("ENTER")));
        assert_eq!(s.job, JobState::Idle);
        // Whitespace is not a path either.
        for c in "   ".chars() {
            press(&mut s, c, None);
        }
        assert!(!s.submit());
        assert_eq!(s.job, JobState::Idle);
    }

    /// Before the first draw — and always while Offline — the recorded
    /// rectangles are zero, so a click can press nothing and the
    /// pointer asks for no hand.
    #[test]
    fn no_rectangle_means_no_control_under_the_pointer() {
        let mut s = offline();
        s.click(10.0, 10.0);
        assert_eq!(s.job, JobState::Idle);
        assert!(!s.button_r.contains(10.0, 10.0));
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

    /// The inputs this widget does not use are INERT, driven through
    /// the table itself: the wheel, the drag, the press entry and the
    /// grid all take nothing and ask nothing of the application — while
    /// the key entry, the one input this panel does use, answers.
    #[test]
    fn unused_inputs_are_inert_and_the_key_entry_answers() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);

        let inst = (API.create)();
        assert!(!inst.is_null());
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        // The wheel and the drag decline with ACTION_NONE — declared
        // no, not silence — so the gesture stays the board's.
        let mut a = untouched();
        (API.wheel)(inst, 1.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_NONE);
        for phase in [DRAG_BEGIN, DRAG_MOVE, DRAG_END] {
            let mut a = untouched();
            (API.drag)(inst, phase, 1.0, 1.0, r, 100.0, 100.0, &mut a);
            assert_eq!(a.kind, ACTION_NONE, "a drag phase must be declined, not eaten");
        }
        // The press entry writes nothing at all: the button is pressed
        // through the click path, and this entry is deliberately not a
        // second one.
        let mut b = untouched();
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(inst, phase, 1.0, 1.0, r, 100.0, 100.0, &mut b);
        }
        assert_eq!(b.kind, u32::MAX, "an entry that does nothing writes nothing");
        // The grid: this widget has none, and saying so writes nothing.
        let (mut cols, mut rows) = (u32::MAX, u32::MAX);
        (API.grid)(inst, &mut cols, &mut rows);
        assert_eq!((cols, rows), (u32::MAX, u32::MAX));

        // The one input the panel DOES use: a character lands in the
        // path box and the answer says so.
        let mut a = untouched();
        assert_eq!((API.key)(inst, 'x' as u32, std::ptr::null(), 0, 0, &mut a), 1);
        assert_eq!(a.kind, ACTION_NONE, "consumed, and nothing asked of the application");
        // TAB is NOT the panel's: the host must still be able to move
        // the focus off it.
        let tab = keys::TAB;
        assert_eq!((API.key)(inst, 0, tab.as_ptr(), tab.len() as u32, 0, &mut a), 0);
        // Nor is a word this build cannot name.
        let unknown = "F13";
        assert_eq!(
            (API.key)(inst, 0, unknown.as_ptr(), unknown.len() as u32, 0, &mut a),
            0
        );

        (API.destroy)(inst);
    }
}
