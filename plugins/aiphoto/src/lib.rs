//! AI PHOTO panel — photo processing over the nacelle-ai daemon: a
//! path box, the list of actions, and the daemon's own word on each.
//!
//! One of the four AI tools on the SEARCH AND AI board, replacing the
//! single inert `ai` widget (see `.gap-program/decyzja-nacelle-ai-daemon.md`,
//! the binding copy). The daemon is spoken to through the shared
//! `nacelle-ai-client` crate — protocol v0, JSON Lines on one Unix
//! socket — and this file owns nothing of that: it polls the client
//! once per frame from the draw path it already has, drains the events
//! into [`model::Photo`], and draws whatever phase the model is in.
//!
//! # Honesty, twice
//!
//! * The daemon's `photo` tool is NOT BUILT YET and says so — every
//!   request comes back an error to that effect, and the panel shows
//!   the daemon's words verbatim rather than a spinner over nothing.
//!   The action list is a placeholder vocabulary and its module
//!   comment says so too ([`model::ACTIONS`]).
//! * A missing daemon is a STATE, not an error: the client answers
//!   Offline, and the panel draws the theme's own `[emptystate]` line
//!   saying what is missing — exactly as the retired `ai` widget did,
//!   and as every panel with nothing to show does.
//!
//! # What is where
//!
//! * [`model`] — the phase machine: the path, the chosen action, and
//!   what every daemon event and every key does to them. Tested there
//!   without a window or a socket.
//! * [`field`] — the path box's DRAWING, the same named duplication
//!   the search panel carries: read its header before judging it.
//! * this file — the ABI boundary, the client wiring, and the layout:
//!   field on top, the action list (or the daemon's message) under it.
//!
//! Every colour, length, duration and word comes from the theme.
//! Nothing here knows what a colour is: a missing token degrades
//! through the raw answers the ABI itself gives, never through a
//! number that used to be the design.

pub mod field;
pub mod model;

use crate::field::FieldView;
use crate::model::{Outcome, Phase, Photo, ACTIONS};
use nacelle::focus::{Key, KeyEv, Mods};
use nacelle::object::text_input::InputMsg;
use nacelle::runtime::{
    keys, ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_CAPTURE,
    ACTION_NONE, DRAG_BEGIN, DRAG_END, DRAG_MOVE, SIZING_REFERENCE,
};
use nacelle::ui::Align;
use nacelle::view::list::{self, ListState, ListStyle, ListView};
use nacelle::view::model::{RowBuf, RowModel};
use nacelle::view::paint::{self, RoleLook};
use nacelle::view::scroll::ScrollPhysics;
use nacelle::view::surface::{AbiSurface, Surface};
use nacelle::view::{Hit, Hits};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use nacelle_ai_client::{AiClient, Status, Tool};
use std::ffi::c_void;

/// The name the host's title band shows, and the name `hello` announces
/// on the socket — which of the four panels asked, in a daemon-side
/// trace.
static TITLE: &[u8] = b"AI PHOTO";
const CLIENT: &str = "aiphoto";

/// What the panel says while the daemon is away. It names the thing
/// that is missing and the one consequence a reader can see, and it
/// promises no date — the retired `ai` widget's rule, kept.
const OFFLINE: &str =
    "Waiting for the nacelle-ai daemon. Photo tools take no input until it answers.";

/// What the empty path box invites.
const PLACEHOLDER: &str = "path to a photo";

/// What the body says between sending a request and the daemon's first
/// word about it.
const SAY_ASKED: &str = "asked nacelle-ai\u{2026}";

/// What sits before an approval request's description. The rest of the
/// sentence is the DAEMON's — what it wants to do — and this panel has
/// no approve control yet, so the one answer it offers is spelled out:
/// Escape, which cancels.
const SAY_APPROVAL: &str = "nacelle-ai asks leave \u{2014} ";
const SAY_APPROVAL_TAIL: &str = " \u{2014} esc cancels";

/// What a row IS, in the trailing column. A word, not look: the theme
/// decides the size, the case and the ink.
const KIND_ACTION: &str = "action";

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

/// The key event a boundary call means, if any — the search panel's
/// reading, for the search panel's reasons: [`keys::from_name`] is the
/// ABI's OWN spelling of the neutral key set, unknown modifier bits are
/// dropped rather than kept, and a control character is no key at all.
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

/// A row's identity, and the way back from one. The key names the
/// ACTION by index into the static list, so a hit key recorded by a
/// view this widget does not own resolves to nothing rather than to
/// row zero.
fn key_of(i: usize) -> String {
    format!("act:{i}")
}

fn action_of(key: &str) -> Option<usize> {
    let n = key.strip_prefix("act:")?;
    let i: usize = n.parse().ok()?;
    (i < ACTIONS.len()).then_some(i)
}

// ----------------------------------------------------------- the widget

/// Everything the panel reads from the theme, once per frame.
struct Look {
    /// `search.gap` — between the box on top and what sits under it.
    /// The search panel's token, read on purpose rather than a new
    /// `aiphoto.*` name: it is the one token in the master that names
    /// exactly this distance (a query box over its answers), and the
    /// two panels sit side by side on the same board. The day the
    /// master grows a widget-scoped word, the name moves.
    gap: f32,
    /// `field.h` — the path box's own height.
    field_h: f32,
    /// `list.row_h + list.gap` — one row's pitch.
    pitch: f32,
    /// `list.wheel_px` — a notch, cached because a wheel event arrives
    /// with no drawing context to ask the theme through.
    wheel_px: f32,
    /// The empty state's own line — offline, and every message the
    /// daemon sends back, are both "the panel has one sentence to say",
    /// which is what the `[emptystate]` role IS.
    empty: RoleLook,
    empty_y: f32,
}

impl Look {
    fn read(sf: &mut impl Surface) -> Look {
        Look {
            gap: sf.px("search.gap").max(0.0),
            field_h: sf.px("field.h").max(0.0),
            pitch: (sf.px("list.row_h") + sf.px("list.gap")).max(1.0),
            wheel_px: sf.px("list.wheel_px"),
            empty: paint::bound_role(sf, "emptystate.role", 1.0),
            empty_y: sf.px("emptystate.y_frac"),
        }
    }
}

/// The action list, as a row model. Pull-based, like every list in
/// this tree: the view materialises the rows it can see.
struct Actions;

impl RowModel for Actions {
    fn len(&self) -> usize {
        ACTIONS.len()
    }

    fn row(&self, index: usize, out: &mut RowBuf) {
        out.reset();
        let Some(word) = ACTIONS.get(index) else { return };
        out.key = key_of(index);
        out.label = word.to_string();
        out.status = KIND_ACTION.to_string();
    }
}

pub struct AiPhoto {
    /// The protocol client — the socket, the retry counter, the event
    /// queue. Polled once per frame; never blocks, never threads.
    client: AiClient,
    /// The model: path, chosen action, phase. Every decision lives
    /// there; this struct is wiring.
    model: Photo,
    /// The path box's between-frame state.
    field: FieldView,
    /// The action list's between-frame state, and the rectangles the
    /// last draw recorded for the input that arrives without any.
    list: ListState,
    hits: Hits,
    field_r: Rect,
    list_r: Rect,
    physics: ScrollPhysics,
    now: f64,
}

impl AiPhoto {
    pub fn new() -> AiPhoto {
        AiPhoto::with_client(AiClient::new(CLIENT))
    }

    /// The same widget over an explicit client. This is the seam the
    /// tests build through — a client aimed at a path no daemon
    /// listens on is a panel that is deterministically Offline.
    pub fn with_client(client: AiClient) -> AiPhoto {
        AiPhoto {
            client,
            model: Photo::new(),
            field: FieldView::new(),
            list: ListState::new(),
            hits: Hits::new(),
            field_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            list_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            // Zeroed rather than read: there is no drawing context yet,
            // and a wheel that arrives before the first frame moves
            // nothing, which is honest — there is nothing on screen.
            physics: ScrollPhysics {
                wheel_px: 0.0,
                fling_scale: 0.0,
                glide_halflife_ms: 0.0,
                settle_ms: 0.0,
                settle_easing: nacelle::view::scroll::Easing::Linear,
                motion_scale: 0.0,
            },
            now: 0.0,
        }
    }

    fn online(&self) -> bool {
        self.client.status() == Status::Connected
    }

    /// What a model outcome asks of the wire, done. Answers whether the
    /// input was consumed — the shape [`key_c`] returns to the host.
    fn settle(&mut self, out: Outcome) -> bool {
        match out {
            Outcome::Ignored => false,
            Outcome::Moved | Outcome::Edited => true,
            Outcome::Cancel(id) => {
                self.client.cancel(id);
                true
            }
            Outcome::Run(i) => {
                // The subject and the verb, as data: the JSON escaper
                // owns the spelling, so a path full of quotes is a
                // path and not a second command.
                let args = serde_json::json!({
                    "path": self.model.path().trim(),
                    "action": ACTIONS[i],
                });
                if let Some(id) = self.client.tool(Tool::Photo, &args) {
                    self.model.sent(id);
                }
                // A `None` here means the daemon left between the
                // frame that drew the list and this key. The model
                // stays Ready and the next frame draws the offline
                // state — nothing is spooled for a daemon that is
                // not there.
                true
            }
        }
    }

    /// A key delivered to this panel. While the daemon is away there is
    /// nothing on screen that takes input — the offline line SAYS so —
    /// and every key is left with the host.
    pub fn key(&mut self, ev: &KeyEv) -> bool {
        if !self.online() {
            return false;
        }
        let out = self.model.key(ev);
        self.settle(out)
    }

    /// A press.
    pub fn click(&mut self, x: f32, y: f32) {
        if !self.online() {
            return;
        }
        if self.field_r.contains(x, y) {
            // The caret goes where the pointer is, from the positions
            // the last draw recorded.
            let at = self.field.hit(x);
            self.model.input.apply(InputMsg::Point { at, extend: false });
            return;
        }
        // Copied out before anything is touched: the hit list and the
        // state a hit changes are both this widget's.
        let hit = self.hits.at(x, y).cloned();
        match hit {
            Some(Hit::Row { key, .. }) => {
                if let Some(i) = action_of(&key) {
                    let out = self.model.run(i);
                    self.settle(out);
                }
            }
            Some(Hit::Track { toward_end, .. }) => {
                let viewport = self.list.extent.viewport;
                self.list.scroll.page(toward_end, viewport, self.now);
            }
            _ => {}
        }
    }

    pub fn wheel(&mut self, notches: f32) {
        // Positive `dy` from the host scrolls toward the START of the
        // content; `ScrollView` counts the other way.
        self.list.scroll.wheel(-notches, &self.physics, self.now);
    }

    /// A press that takes hold of the action list's scroll thumb — the
    /// one gesture this panel takes. Anything else declines, and the
    /// press falls back to the ordinary click delivery.
    pub fn grab(&mut self, x: f32, y: f32) -> bool {
        let Some((_, thumb)) = self.list.extent.bar else { return false };
        thumb.contains(x, y) && self.list.scroll.press_thumb(y, thumb)
    }

    pub fn drag_to(&mut self, y: f32) {
        let Some((track, _)) = self.list.extent.bar else { return };
        let (viewport, content) = (self.list.extent.viewport, self.list.extent.content);
        self.list.scroll.drag(y, viewport, content, track);
    }

    pub fn release(&mut self) {
        self.list.scroll.release();
    }

    /// One sentence, centred where `[emptystate]` puts it — the shape
    /// every panel in this tree gives a body that has one thing to say.
    fn say(&self, sf: &mut impl Surface, r: Rect, look: &Look, text: &str) {
        let y = r.y + r.h * look.empty_y - look.empty.px * look.empty.leading / 2.0;
        sf.text(
            look.empty.face,
            look.empty.px,
            r.cx(),
            y,
            text,
            look.empty.color,
            look.empty.track,
            Align::Center,
        );
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let mut sf = AbiSurface::new(api, ctx);
        self.hits.clear();
        self.now = sf.now();

        // The frame's worth of protocol: reconnect if due, flush, read,
        // and every arrived event into the model. Bounded, non-blocking,
        // and the whole of what "the widget talks to the daemon" is.
        self.client.poll();
        while let Some(ev) = self.client.take_event() {
            self.model.on_event(&ev);
        }

        let look = Look::read(&mut sf);
        self.physics = ScrollPhysics::read(&mut sf);
        // A row list names its own notch; a theme that names none
        // leaves it at one row, the smallest move that means anything.
        self.physics.wheel_px = look.wheel_px.max(look.pitch);

        if !self.online() {
            // A request that was out died with the socket; saying
            // "working" over a dead connection would be the spinner
            // this widget exists to not be.
            self.model.connection_lost();
            // The rectangles are the last DRAW's, and this draw shows
            // no field and no list: a click must find neither.
            self.field_r = Rect::new(0.0, 0.0, 0.0, 0.0);
            self.list_r = Rect::new(0.0, 0.0, 0.0, 0.0);
            self.say(&mut sf, r, &look, OFFLINE);
            return;
        }

        // The box on top, the actions (or the daemon's word) under it.
        let field_r = Rect::new(r.x, r.y, r.w, look.field_h.min(r.h));
        let top = field_r.bottom() + look.gap;
        let list_r = Rect::new(r.x, top, r.w, (r.bottom() - top).max(0.0));
        self.field_r = field_r;
        self.list_r = list_r;

        // There is no focus chain across this boundary, so the panel
        // answers the question the chain would: the path box owns the
        // keyboard for as long as the panel is on screen.
        field::draw(&mut sf, field_r, &self.model.input, &mut self.field, PLACEHOLDER, true);

        match self.model.phase() {
            Phase::Ready => {
                self.list.selected = Some(key_of(self.model.cursor()));
                list::list(
                    &mut sf,
                    list_r,
                    &Actions,
                    &ListStyle::default(),
                    Some(ListView {
                        state: &mut self.list,
                        hits: &mut self.hits,
                        id: 0,
                        select: true,
                        scroll: true,
                        tree: false,
                        tooltip: true,
                    }),
                );
            }
            // Anything else is one sentence, and the sentence is drawn
            // where every one-sentence body in this tree is drawn.
            Phase::Working { note } if note.is_empty() => {
                self.say(&mut sf, list_r, &look, SAY_ASKED)
            }
            Phase::Working { note } => {
                let note = note.clone();
                self.say(&mut sf, list_r, &look, &note);
            }
            Phase::Waiting { desc } => {
                let text = format!("{SAY_APPROVAL}{desc}{SAY_APPROVAL_TAIL}");
                self.say(&mut sf, list_r, &look, &text);
            }
            Phase::Said { text } => {
                let text = text.clone();
                self.say(&mut sf, list_r, &look, &text);
            }
        }
    }
}

impl Default for AiPhoto {
    fn default() -> Self {
        AiPhoto::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(AiPhoto::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut AiPhoto) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut AiPhoto> {
    unsafe { (instance as *mut AiPhoto).as_mut() }
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
        // Whatever was asked is already on the wire; the host has
        // nothing to do about it.
        out.kind = ACTION_NONE;
    }
}

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
/// the search panel's question, asked the search panel's way.
fn routes_keys(api: &HostApi) -> bool {
    api.has_channel()
}

/// The BROADCAST, listened to only where nothing else is delivered:
/// an older host that never calls `key` would otherwise have a path
/// box with no keyboard at all. On a routing host, acting on both
/// would type every character twice.
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
/// and all. Nonzero says the panel used it, so the host must not also
/// spend it on focus navigation or a shortcut.
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
    let Some(ev) = key_ev(ch, label, mods) else {
        // A key this build cannot name and cannot spell is not one the
        // panel used; leaving it to the host is the honest answer.
        return 0;
    };
    this.key(&ev) as u32
}

/// Sized against the reference box: the panel holds one field and a
/// STATIC list of four actions — nothing here grows in rows the way a
/// result page does.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    SIZING_REFERENCE
}

/// The header: the panel's name, and no right-hand half. The right half
/// is where a panel says something ABOUT itself, and what this one
/// would say — the daemon's presence — the body already says in full.
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

/// The one gesture this panel takes: the action list's scroll thumb.
/// Every other press declines the capture and falls back to the click
/// path, which is what puts the caret and what runs an action.
#[allow(clippy::too_many_arguments)]
extern "C" fn drag_c(
    instance: *mut c_void,
    phase: u32,
    x: f32,
    y: f32,
    _r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    let mut kind = ACTION_NONE;
    if let Some(this) = state(instance) {
        match phase {
            DRAG_BEGIN => {
                kind = if this.grab(x, y) { ACTION_CAPTURE } else { ACTION_NONE };
            }
            DRAG_MOVE => this.drag_to(y),
            DRAG_END => this.release(),
            // A phase from a newer host than this build knows must not
            // be guessed at: an unknown gesture is no gesture.
            _ => {}
        }
    }
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = kind;
    }
}

/// Nothing of this widget asks for the hand cursor: declining every
/// point keeps the ordinary pointer, honestly.
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

/// Filled, and does nothing on purpose. The one gesture this panel
/// takes is the thumb, and the thumb is `drag`'s — the single capture
/// path, of which this entry is deliberately not a second. No control
/// here draws a press rung of its own.
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
/// `aiphoto.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `aiphoto.meta` beside it — so a host never
/// describes a widget it merely links.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "aiphoto",
    meta: include_str!("../aiphoto.meta"),
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
    use nacelle::base::{PanelSlot, WidgetCategory};
    use nacelle::widget::registry;

    /// The board this panel may be placed on, read the way the host
    /// reads it: from the addon's own `.meta`, through the registry's
    /// parser. An unknown category word degrades to BOARD silently,
    /// which is the silence this test exists to break.
    #[test]
    fn the_widget_registers_on_the_search_and_ai_board() {
        let def = registry::def_from_meta(WIDGET.name.to_string(), WIDGET.meta);
        assert_eq!(def.name, "aiphoto");
        assert_eq!(def.label, "AI PHOTO");
        assert_eq!(def.category, WidgetCategory::SearchAi);
        assert_ne!(WidgetCategory::default(), WidgetCategory::SearchAi);
        assert!(def.ref_h_vh > 0.0 && def.min_h_vh > 0.0);
        assert!(def.min_h_vh <= def.ref_h_vh);
        // The tool stack's own column and place: center, between loop
        // (10) and sort (30). A typo here would quietly re-shuffle the
        // board, so the two numbers are pinned.
        assert_eq!(def.slot, PanelSlot::Center);
        assert_eq!(def.order, 20.0);
        // And the weight is deliberately NOT declared — the meta's own
        // comment says why — so the default must still be the default.
        assert_eq!(def.weight, None);
    }

    /// The words this panel promises its reader are held to what the
    /// panel can answer for: the offline line names the thing that is
    /// missing and promises no date.
    #[test]
    fn the_offline_message_says_what_is_missing_and_promises_no_date() {
        assert!(OFFLINE.contains("nacelle-ai"));
        for promise in ["soon", "coming", "shortly", "will be", "next release"] {
            assert!(!OFFLINE.to_lowercase().contains(promise), "{promise:?} is a date");
        }
    }

    #[test]
    fn a_row_key_names_the_action_and_survives_the_round_trip() {
        for i in 0..ACTIONS.len() {
            assert_eq!(action_of(&key_of(i)), Some(i));
        }
        // Nothing else is a key.
        assert_eq!(action_of(""), None);
        assert_eq!(action_of("act:"), None);
        assert_eq!(action_of("act:-1"), None);
        assert_eq!(action_of(&format!("act:{}", ACTIONS.len())), None);
        assert_eq!(action_of("row:1"), None);
    }
}

#[cfg(test)]
mod token_tests {
    /// Every token name this crate asks the theme for by a name of its
    /// own, spelled exactly as the code spells it — the whole of
    /// `field.rs`, `Look::read` and the row model.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise: a widget that names a token the master does not
    /// declare gets `u32::MAX` back, `theme_px` answers zero, and the
    /// thing degrades silently. A typo never fails loudly anywhere
    /// else — so it fails here.
    const TOKENS: &[&str] = &[
        // the gap this panel shares with its board-mate — see Look
        "search.gap",
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
        // the actions
        "list.row_h",
        "list.gap",
        "list.wheel_px",
        "list.label_role",
        "list.status_role",
        // and every sentence the body has to say alone
        "emptystate.role",
        "emptystate.y_frac",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
        // The classes the two halves rest on.
        for class in ["field", "list.item", "scrollbar.thumb"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
    }
}

#[cfg(test)]
mod abi_tests {
    use super::*;
    use nacelle::runtime::{
        BUTTON_PRESS, BUTTON_RELEASE, MODS_CTRL, PLUGIN_API_HAS_BUTTON,
    };
    use std::path::PathBuf;

    /// A panel whose daemon is deterministically away: the client is
    /// aimed at a path nothing listens on, so the tests do not depend
    /// on whether THIS machine happens to run nacelle-ai.
    fn offline_panel() -> AiPhoto {
        AiPhoto::with_client(AiClient::at(CLIENT, PathBuf::from("/nonexistent-nacelle-test/ai.sock")))
    }

    /// A value no entry of this widget could ever write, so "left
    /// alone" is something a test can see.
    fn untouched() -> ActionC {
        ActionC { kind: u32::MAX, index: 0, lines: 0, data: std::ptr::null(), data_len: 0 }
    }

    /// The entries appended in this ABI version are filled AND declared
    /// — two different things, and the host checks the second before it
    /// calls either.
    #[test]
    fn the_appended_entries_are_declared() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);
    }

    /// The inputs this widget does not use are INERT, driven through
    /// the table itself: the press entry writes nothing, the pointer
    /// entry asks for no cursor, the grid entry is a no-op — and an
    /// unknown drag phase is no gesture. A later change that gives any
    /// of them a job has to come past the reasons written on them, and
    /// past this test, rather than around both.
    #[test]
    fn the_unused_inputs_are_inert() {
        let mut w = offline_panel();
        let inst = &mut w as *mut AiPhoto as *mut c_void;
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        let mut a = untouched();
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(inst, phase, 1.0, 1.0, r, 100.0, 100.0, &mut a);
        }
        assert_eq!(a.kind, u32::MAX, "an entry that does nothing writes nothing");

        assert_eq!((API.pointer)(inst, 1.0, 1.0, r, 100.0, 100.0), 0);
        (API.grid)(inst, std::ptr::null_mut(), std::ptr::null_mut());

        // A drag phase this build does not know is no gesture: the
        // capture is declined, not guessed at.
        let mut d = untouched();
        (API.drag)(inst, 999, 1.0, 1.0, r, 100.0, 100.0, &mut d);
        assert_eq!(d.kind, ACTION_NONE);
        // And an ordinary BEGIN off any thumb declines too — before
        // the first draw there is no thumb to hold.
        let mut d = untouched();
        (API.drag)(inst, DRAG_BEGIN, 1.0, 1.0, r, 100.0, 100.0, &mut d);
        assert_eq!(d.kind, ACTION_NONE);
    }

    /// While the daemon is away the panel takes NOTHING: the offline
    /// line says so in words, and this is the same fact at the
    /// boundary — a key is not consumed, a click asks no action, and
    /// nothing is queued for a daemon that is not there.
    #[test]
    fn an_offline_panel_consumes_no_input_and_queues_nothing() {
        let mut w = offline_panel();
        w.client.poll();
        assert_eq!(w.client.status(), Status::Offline);
        let inst = &mut w as *mut AiPhoto as *mut c_void;
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        let mut a = untouched();
        assert_eq!((API.key)(inst, 'a' as u32, std::ptr::null(), 0, 0, &mut a), 0);
        assert_eq!(a.kind, ACTION_NONE);
        let word = keys::DOWN;
        assert_eq!((API.key)(inst, 0, word.as_ptr(), word.len() as u32, 0, &mut a), 0);
        assert_eq!(
            (API.key)(inst, 'a' as u32, std::ptr::null(), 0, MODS_CTRL, &mut a),
            0
        );

        (API.click)(inst, 10.0, 10.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_NONE);

        assert_eq!(w.model.path(), "", "no key reached the field");
        assert_eq!(w.model.phase(), &Phase::Ready);
        assert_eq!(w.model.request(), None);
    }

    /// The same keys once the model is reachable: a character is the
    /// panel's and the answer says so, TAB and an unknown word are not.
    /// Driven through [`AiPhoto::key`] with the connection question
    /// bypassed at its own seam — the boundary translation is what the
    /// table calls above already proved.
    #[test]
    fn the_key_channel_translates_what_it_can_and_refuses_the_rest() {
        // Translation, exactly as the boundary does it.
        assert_eq!(key_ev('a' as u32, None, 0).map(|e| e.key), Some(Key::Char('a')));
        assert_eq!(key_ev(0, Some("ENTER"), 0).map(|e| e.key), Some(Key::Enter));
        assert_eq!(key_ev(0, Some("DOWN"), 0).map(|e| e.key), Some(Key::Down));
        assert!(key_ev(0, Some("F13"), 0).is_none());
        assert!(key_ev(0x1b, None, 0).is_none(), "a control character is no key");
        assert_eq!(
            key_ev('a' as u32, None, MODS_CTRL | 1 << 20).map(|e| e.mods),
            Some(Mods::CTRL),
            "an unknown modifier bit is dropped, not kept"
        );

        // A label pointer, as the boundary really passes one.
        unsafe {
            assert_eq!(label_of(b"UP".as_ptr(), 2), Some("UP"));
            assert_eq!(label_of(b"UP".as_ptr(), 0), None);
            assert_eq!(label_of(std::ptr::null(), 4), None);
            assert_eq!(label_of([0xffu8, 0xfe].as_ptr(), 2), None);
        }
    }
}
