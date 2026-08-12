//! AI panel — the assistant's place on the SEARCH AND AI board, held
//! open and deliberately doing nothing.
//!
//! The assistant itself is `nacelle-ai`, a separate repository still
//! being written. Until it answers, this widget exists so that the
//! board it belongs on is a board and not a hole, and it is INERT by
//! design: it reads no key, takes no click, opens no socket and runs
//! nothing between frames. Its whole cost is one wrapped message per
//! draw.
//!
//! # Why there is no text field
//!
//! Because a field that accepts characters and does nothing with them
//! is worse than no field at all. A reader who types into one cannot
//! tell a program that is BROKEN from a program that is UNFINISHED, and
//! the panel has no way to tell them afterwards. So there is nothing to
//! type into, and the panel says in words which of the two it is. The
//! empty state IS the widget.
//!
//! # What the panel is made of
//!
//! Nothing but its content. The frame around it — the ring, the title
//! band that reads AI, the content box this file is handed — is the
//! HOST's panel container, which every widget gets by naming itself
//! through [`PluginApi::chrome`] (see `nacelle::object::panel`: "the
//! widget is then handed the CONTENT BOX and draws content, never
//! chrome"). Inside that box this file draws the theme's own empty
//! state: the `[emptystate]` group says which type role the "nothing
//! here" message is set in (`role`) and where in the box it sits
//! (`y_frac`), and the role says everything else — size, tracking,
//! leading, case, face and ink. There is not a colour, a length or a
//! position in this file; a missing token degrades through the raw
//! answers the ABI itself gives (no ink, zero lengths), never through
//! a number that used to be the design.
//!
//! # What lands here when nacelle-ai is ready
//!
//! Three things, and they replace [`Ai::draw`]'s body rather than
//! joining it:
//!
//! 1. the CONVERSATION VIEW — the turns so far as a scrolling column,
//!    which is a list and should be built from the toolkit's scrolling
//!    view (`nacelle::view::scroll`) exactly as the file browser is,
//!    not from a second copy of that arithmetic;
//! 2. STREAMING — a partial answer growing a token at a time, which is
//!    the first thing in this tree that redraws because something
//!    OFFSCREEN moved, and therefore the first that needs a frame
//!    request rather than a poll;
//! 3. the BACKEND INDICATOR — which of the two answered, the local
//!    model on Ollama or Claude over the network. It belongs in the
//!    title band's right half (`ChromeC::right`, where the file browser
//!    puts its cwd and the launcher its count), because who is
//!    answering is a fact about the panel and not about any one turn.
//!
//! An input field arrives WITH the first of those three and not before:
//! see above for why.

use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_NONE,
    SIZING_REFERENCE,
};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use std::ffi::c_void;

/// The name the host's title band shows. English in the code, as every
/// string in this tree is: what a user reads is the theme's and the
/// locale's business, not this file's.
static TITLE: &[u8] = b"AI";

/// What the panel is waiting for, said outright.
///
/// It names the thing that is missing (`nacelle-ai`) and the one
/// consequence a reader can see (nothing here takes input), and it
/// promises no date, because this file has no way of knowing one and a
/// panel that guesses at one is a panel that will be wrong.
const MESSAGE: &str =
    "Waiting for the nacelle-ai backend. This panel accepts no input until that backend answers.";

/// The font slots, as the host numbers them — the theme's own
/// `FACE_UI = 0` and `FACE_MONO = 1`. The ABI carries these two and
/// clamps anything past them, so a slot is chosen by the WORD a role's
/// `face` names and never by an index into the theme's eight faces.
const FONT_UI: u32 = 0;
const FONT_MONO: u32 = 1;

/// No ink at all, for the host that predates ABI 5 and cannot be asked
/// for a colour.
///
/// Not a grey: a chosen grey would be a design decision taken where the
/// theme could not be reached, and this program has none of those. It
/// is never seen either way — [`EmptyLook::raw`] pairs it with a type
/// size of zero, and type of no size draws nothing.
const NO_INK: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    // Written once during attach, before any other call, and never
    // again; every read afterwards is on the frame thread.
    unsafe { HOST }
}

// ------------------------------------------------------------------ theme

fn token(api: &HostApi, name: &str) -> u32 {
    (api.theme_token)(name.as_ptr(), name.len() as u32)
}

/// The WORD an enum token currently resolves to — ABI 6's appended
/// `theme_enum_word` entry. Init-time like `theme_token`: asked when the
/// ids are resolved, cached for the epoch, never in the draw loop. An
/// empty answer — a host whose table ends before the entry, a missing
/// token, a token with no word — degrades exactly like MISSING.
fn enum_word(api: &HostApi, ctx: *mut c_void, id: u32) -> String {
    if !api.has_theme_enum_word() || id == u32::MAX {
        return String::new();
    }
    let mut buf = [0u8; 64];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
}

/// The font slot a type role's `face` token names.
///
/// A face is an OPEN word set — `ui`, `mono`, `ui_medium`, `display` are
/// all faces the theme declares — so it is read as a WORD rather than as
/// an index: the boundary numbers two slots and clamps anything past
/// them, which would turn `display` into monospace. A mono face answers
/// the mono slot; every other face answers the interface slot, which is
/// where the boundary puts them all anyway.
fn face_slot(api: &HostApi, ctx: *mut c_void, id: u32) -> u32 {
    if enum_word(api, ctx, id).starts_with("mono") {
        FONT_MONO
    } else {
        FONT_UI
    }
}

/// A type role's case transform, applied here because the text entry
/// draws bytes as given. The indices are the schema's declared order —
/// every `*.case` declares `enum: none | upper | lower | smallcaps`,
/// and `theme_enum` indexes that list. Smallcaps needs per-glyph sizes
/// only the host's font system has; through a single text call the
/// nearest honest reading is capitals.
fn recase(word: u32, s: &str) -> String {
    match word {
        1 | 3 => s.to_uppercase(), // upper | smallcaps
        2 => s.to_lowercase(),     // lower
        _ => s.to_string(),        // none, or a word this build predates
    }
}

/// Token ids the empty state draws from, resolved by NAME once per
/// epoch.
///
/// The `[emptystate]` group is two keys and the second is a REFERENCE:
/// `role` names a type role rather than a member of a closed list, so
/// it is read as a word and the role's own family is resolved from it —
/// the same shape the file browser resolves `shape.icon_tile.glow` by.
/// This is what keeps "which type an empty state is set in" a sentence
/// the theme writes once, for every panel that has nothing to show,
/// instead of a pick each widget makes for itself.
struct EmptyTheme {
    epoch: u32,
    y_frac: u32, // emptystate.y_frac
    // type.<emptystate.role>.* — the role the group above names
    size: u32,
    min: u32,
    tracking: u32,
    leading: u32,
    case: u32,
    fg: u32,
    /// The slot the role's `face` names, read WITH the ids because a
    /// face is a word and reading words is init-time work.
    font: u32,
}

/// The name of one token of the role `emptystate.role` binds to.
///
/// `None` for a master that binds no role at all, which leaves every id
/// MISSING and every accessor on the engine's raw default: zero lengths,
/// which draw nothing. Substituting a role of this file's choosing would
/// be this file deciding the look.
fn role_token(role: &str, suffix: &str) -> Option<String> {
    if role.is_empty() {
        return None;
    }
    Some(format!("type.{role}.{suffix}"))
}

impl EmptyTheme {
    fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> EmptyTheme {
        let role = enum_word(api, ctx, token(api, "emptystate.role"));
        let of_role = |suffix: &str| match role_token(&role, suffix) {
            Some(name) => token(api, &name),
            None => u32::MAX,
        };
        EmptyTheme {
            epoch,
            y_frac: token(api, "emptystate.y_frac"),
            size: of_role("size"),
            min: of_role("min_px"),
            tracking: of_role("tracking"),
            leading: of_role("leading"),
            case: of_role("case"),
            fg: of_role("fg"),
            font: face_slot(api, ctx, of_role("face")),
        }
    }
}

/// The values one frame draws with, read fresh from the resolved ids.
/// Colours and lengths only — nothing here is arithmetic on anything.
struct EmptyLook {
    /// `emptystate.y_frac`: where in the content box the message sits,
    /// as a fraction of the box's height.
    y_frac: f32,
    px: f32,
    tracking: f32,
    leading: f32,
    case: u32,
    font: u32,
    ink: ColorC,
}

impl EmptyLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// No ink, zero lengths — type of no size draws nothing, which is
    /// the same undesigned raw an empty theme gives.
    fn raw() -> EmptyLook {
        EmptyLook {
            y_frac: 0.0,
            px: 0.0,
            tracking: 0.0,
            leading: 1.0,
            case: 0,
            font: FONT_UI,
            ink: NO_INK,
        }
    }

    fn read(api: &HostApi, ctx: *mut c_void, t: &EmptyTheme) -> EmptyLook {
        let px = |id| (api.theme_px)(ctx, id);
        EmptyLook {
            y_frac: px(t.y_frac),
            px: px(t.size).max(px(t.min)),
            tracking: px(t.tracking),
            // A leading below one line would stack the lines on top of
            // each other; the role declares 1.0 .. 2.0 and the floor is
            // what a nonsense value degrades to, not a chosen pitch.
            leading: px(t.leading).max(1.0),
            case: (api.theme_enum)(ctx, t.case),
            font: t.font,
            ink: (api.theme_color)(ctx, t.fg),
        }
    }
}

/// Whether the theme has said enough for the message to be readable.
///
/// Type of no size, or ink of no alpha, is a master that has said
/// nothing about empty states — and the honest answer to that is to
/// draw nothing, exactly as a page with no stylesheet is unstyled
/// rather than styled by its browser. It is also the property that
/// makes "no hardcoded values" visible: were a size or a colour written
/// into this file, a silent theme would still put a message on the
/// screen.
fn legible(look: &EmptyLook) -> bool {
    look.px > 0.0 && look.ink.a > 0.0
}

// ----------------------------------------------------------------- layout

/// One laid-out line of the message: the text, and the top of its line
/// box.
#[derive(Clone, Debug, PartialEq)]
struct Line {
    text: String,
    y: f32,
}

/// The message broken to fit `max_w`, greedily, at spaces.
///
/// `width_of` is the host's own measurement, passed in rather than
/// called here, so that the wrap can be exercised without a host to
/// measure through — and so that nothing in this function knows what a
/// font is.
///
/// A line always takes its first word however narrow the box is: a word
/// that fits nowhere still has to go somewhere, and a line allowed to
/// stay empty is another name for a loop that does not end. A `max_w`
/// that is zero, negative or not a number therefore gives one word per
/// line rather than nothing at all.
fn wrap(message: &str, max_w: f32, width_of: impl Fn(&str) -> f32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // The line as it WOULD be with one more word on it. One buffer for
    // the whole wrap rather than a string per word: this runs on every
    // frame the panel is on screen, and a panel that does nothing should
    // cost nothing to do it.
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

/// Where each line of the message goes in `area`.
///
/// The block is centred on `emptystate.y_frac` of the box's height —
/// the same reading the launcher grid gives that token for its own one
/// line, so a panel with nothing to show and a panel with nothing
/// installed put their message in the same place. The pitch between two
/// lines is the role's own `leading`, because line height is what
/// leading IS; nothing here invents a gap.
///
/// Two things it refuses to do, both for the same reason — a widget
/// draws inside its content box and nowhere else:
///
/// * the block never starts above the box, so a panel too short to
///   centre the message shows its BEGINNING rather than its middle;
/// * a line whose box does not fit whole is not emitted, so the last
///   line of a squeezed panel cannot paint over the neighbour below it.
fn lay_out(
    area: Rect,
    look: &EmptyLook,
    message: &str,
    width_of: impl Fn(&str) -> f32,
) -> Vec<Line> {
    let lines = wrap(message, area.w, width_of);
    let pitch = look.px * look.leading;
    let block = lines.len() as f32 * pitch;
    let top = (area.y + area.h * look.y_frac - block / 2.0).max(area.y);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| Line { text, y: top + i as f32 * pitch })
        .filter(|l| l.y >= area.y && l.y + pitch <= area.bottom())
        .collect()
}

// ------------------------------------------------------------- the widget

/// The panel. One cached set of token ids and nothing else: there is no
/// state to keep, because there is nothing this widget can be in the
/// middle of.
pub struct Ai {
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<EmptyTheme>,
}

impl Ai {
    pub fn new() -> Self {
        Ai { theme: None }
    }

    /// This frame's theme values. Ids are cached across frames; the
    /// values are read fresh, because they are what a mood or a resize
    /// changes.
    fn look(&mut self, api: &HostApi, ctx: *mut c_void) -> EmptyLook {
        // ABI 5 is where the token entries live. attach() refuses an
        // older host outright, so this branch is belt and braces for the
        // day the check moves — an old table simply ends before these
        // entries do.
        if api.abi_version < 5 {
            return EmptyLook::raw();
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|t| t.epoch) != Some(epoch) {
            self.theme = Some(EmptyTheme::resolve(api, ctx, epoch));
        }
        match &self.theme {
            Some(t) => EmptyLook::read(api, ctx, t),
            None => EmptyLook::raw(),
        }
    }

    /// The empty state, and nothing else. When nacelle-ai lands this
    /// body is REPLACED by the conversation view, the streaming answer
    /// and the backend indicator described at the head of this file —
    /// this is a deliberate blank, not a skeleton somebody forgot.
    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let look = self.look(api, ctx);
        // Measuring and wrapping a message that would draw as nothing is
        // work with no reader, so it is not done.
        if !legible(&look) {
            return;
        }
        let spacing = look.px * look.tracking;
        let message = recase(look.case, MESSAGE);
        let lines = lay_out(r, &look, &message, |s| {
            (api.measure)(ctx, look.font, look.px, s.as_ptr(), s.len() as u32, spacing)
        });
        for line in lines {
            (api.text)(
                ctx,
                look.font,
                look.px,
                r.cx(),
                line.y,
                line.text.as_ptr(),
                line.text.len() as u32,
                look.ink,
                spacing,
                1, // centred, like every other panel's empty state
            );
        }
    }
}

impl Default for Ai {
    fn default() -> Self {
        Ai::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Ai::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Ai) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Ai> {
    unsafe { (instance as *mut Ai).as_mut() }
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

/// A click lands on nothing. Answering `ACTION_NONE` is not the same as
/// ignoring the press: it tells the host there is no action to take, so
/// the press stays the panel container's — which is what moves, focuses
/// and closes the panel like any other.
extern "C" fn click_c(
    _: *mut c_void,
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

/// One message does not scroll. The conversation view will, and it will
/// scroll through the toolkit's own `view::scroll` rather than through
/// arithmetic invented here.
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

/// The physical keyboard is not listened to. There is nowhere for a
/// character to go, and a widget that quietly consumed one would be the
/// dead text field this panel exists to avoid.
extern "C" fn key_feedback_c(_: *mut c_void, _: u32, _: *const u8, _: u32) {}

/// Sized against the reference box on both axes: what the panel holds
/// is one message, whose height is the message's and not the width's,
/// so there is nothing here that grows in rows.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    SIZING_REFERENCE
}

/// The header, as chrome: the panel's name, and no right-hand half.
///
/// The right half is where a panel says something ABOUT itself — the
/// file browser's cwd, the launcher's count — and this one has nothing
/// true to put there yet. It is where the backend indicator goes when
/// nacelle-ai can be asked which of the two answered; inventing a
/// status line before there is a status would be the title band making
/// the same promise the body refuses to make.
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
/// the ordinary click path.
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

/// Nothing of this widget asks for the hand cursor: it is drawn, not
/// operated. Declining every point is the honest answer, and the panel
/// keeps the ordinary pointer.
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
};

/// This addon, for a host that LINKS the crate in instead of loading
/// `ai.so` from the addons directory. The name and the metadata are the
/// addon's own — the same string the file would be called and the very
/// bytes of `ai.meta` beside it — so a host never describes a widget it
/// merely links: it hands this constant over whole and learns
/// everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "ai",
    meta: include_str!("../ai.meta"),
    attach: builtin_attach,
};

/// In-process attach for a host that links this crate statically. The
/// dlopen attach below goes through `runtime::attach`, which flips the
/// toolkit into forwarding mode — correct for a plugin carrying its own
/// copy of the toolkit, and exactly wrong when this copy IS the host's.
/// So the built-in path only takes the interface and answers with the
/// table.
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
        assert_eq!(def.name, "ai");
        assert_eq!(def.label, "AI");
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
    /// Every token name this crate asks the theme for by a name of its
    /// own, spelled exactly as the code spells it. The role's family is
    /// NOT here — it is spelled `type.<word>.*` at run time, and the
    /// test below chases the word the master actually binds.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise. A widget that names a token the master does not
    /// declare gets `u32::MAX` back, `theme_px` answers zero, and the
    /// thing degrades silently: a message of no size looks exactly like
    /// an empty state nobody implemented. A typo would therefore never
    /// fail loudly anywhere else — so it fails here.
    const TOKENS: &[&str] = &["emptystate.role", "emptystate.y_frac"];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
    }

    /// `emptystate.role` names a type ROLE, and a role that does not
    /// exist is a message with no size, no ink and no leading — drawn
    /// as nothing, reported as nothing. So the binding is followed to
    /// the end here, exactly as `EmptyTheme::resolve` follows it.
    #[test]
    fn the_role_the_empty_state_is_bound_to_is_a_role_the_master_declares() {
        nacelle::theme::load();
        let id = nacelle::theme::id("emptystate.role").expect("emptystate.role");
        let role = nacelle::theme::enum_word_of(id).expect("emptystate.role names no word");
        assert!(!role.is_empty());
        for suffix in ["size", "min_px", "tracking", "leading", "case", "fg", "face"] {
            let name = format!("type.{role}.{suffix}");
            assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
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

    /// An ink a theme really answered with. Test data, not a default:
    /// it exists so a case can say "the theme spoke" and be told apart
    /// from [`NO_INK`], which says the opposite.
    const SOME_INK: ColorC = ColorC { r: 0.8, g: 0.9, b: 1.0, a: 1.0 };

    /// A look as the theme would hand one over — every field a value
    /// the master really has a token for, none of them written into the
    /// code under test.
    fn look(y_frac: f32, px: f32, leading: f32) -> EmptyLook {
        EmptyLook { y_frac, px, tracking: 0.0, leading, case: 0, font: FONT_UI, ink: SOME_INK }
    }

    #[test]
    fn the_message_sits_where_emptystate_y_frac_says_and_nowhere_else() {
        let area = Rect::new(0.0, 100.0, 2000.0, 400.0);
        // Wide enough for the whole message on one line, so the block is
        // one pitch tall and the arithmetic is readable.
        let l = look(0.4, 20.0, 1.2);
        let lines = lay_out(area, &l, MESSAGE, width_of);
        assert_eq!(lines.len(), 1);
        let pitch = 20.0 * 1.2;
        assert!((lines[0].y - (100.0 + 400.0 * 0.4 - pitch / 2.0)).abs() < 0.01);

        // Move the token and the message moves with it — the position
        // is READ, not written into this file. Same box, same type,
        // different `y_frac`.
        let higher = lay_out(area, &look(0.1, 20.0, 1.2), MESSAGE, width_of);
        let lower = lay_out(area, &look(0.9, 20.0, 1.2), MESSAGE, width_of);
        assert!(higher[0].y < lines[0].y, "y_frac 0.1 must sit above 0.4");
        assert!(lower[0].y > lines[0].y, "y_frac 0.9 must sit below 0.4");
        // And so does the type: a larger role is a lower first baseline
        // for the same single line, because the line box is taller.
        let big = lay_out(area, &look(0.4, 40.0, 1.2), MESSAGE, width_of);
        assert!(big[0].y < lines[0].y);
    }

    #[test]
    fn two_lines_are_a_leading_apart_and_a_leading_only() {
        // Narrow enough to force a wrap, tall enough to hold what it
        // forces.
        let area = Rect::new(0.0, 0.0, 400.0, 4000.0);
        let l = look(0.5, 16.0, 1.5);
        let lines = lay_out(area, &l, MESSAGE, width_of);
        assert!(lines.len() > 1, "a 400 px box cannot hold this message on one line");
        let pitch = 16.0 * 1.5;
        for pair in lines.windows(2) {
            assert!((pair[1].y - pair[0].y - pitch).abs() < 0.01);
        }
        // Every line fits the box it was measured against, first word
        // apart — the wrap is a wrap and not a truncation, so no word of
        // the message is lost.
        let drawn: String = lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join(" ");
        assert_eq!(drawn.split_whitespace().count(), MESSAGE.split_whitespace().count());
    }

    #[test]
    fn nothing_is_drawn_outside_the_content_box() {
        // A box with room for two lines out of the several this message
        // needs: the ones that fit are drawn, the rest are not, and none
        // of them starts above the box.
        let l = look(0.5, 16.0, 1.5);
        let area = Rect::new(10.0, 20.0, 300.0, 60.0);
        for line in lay_out(area, &l, MESSAGE, width_of) {
            assert!(line.y >= area.y);
            assert!(line.y + 16.0 * 1.5 <= area.bottom());
        }
    }

    /// The one test that has to hold for a widget nobody is watching:
    /// whatever rectangle the layout engine ends up handing over — a
    /// collapsed panel, a wall-sized one, a degenerate one — drawing it
    /// is arithmetic and not a crash.
    ///
    /// This IS the drawing path's whole panic surface. [`Ai::draw`] adds
    /// three things to what is exercised here — the [`legible`] guard,
    /// one `recase`, and a host call per line — and none of them can
    /// fail on a rectangle; every number that a frame's geometry could
    /// make nonsense is computed by [`lay_out`], which is why it is a
    /// free function taking its measurement rather than a method
    /// reaching for a host.
    #[test]
    fn drawing_an_extreme_box_is_arithmetic_and_never_a_panic() {
        let looks = [
            look(0.4, 16.0, 1.2),
            look(0.0, 0.0, 1.0),    // a master that declares nothing
            look(1.0, 400.0, 2.0),  // type taller than most panels
            look(-1.0, 16.0, 1.2),  // a nonsense fraction
            look(f32::NAN, 16.0, 1.2),
        ];
        let areas = [
            Rect::new(0.0, 0.0, 0.0, 0.0),
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Rect::new(0.0, 0.0, -500.0, -500.0),
            Rect::new(-1e6, -1e6, 1e6, 1e6),
            Rect::new(0.0, 0.0, 1e9, 1e9),
            Rect::new(0.0, 0.0, f32::NAN, f32::NAN),
            Rect::new(0.0, 0.0, f32::INFINITY, f32::INFINITY),
        ];
        for l in &looks {
            for area in areas {
                let lines = lay_out(area, l, MESSAGE, width_of);
                // Not one line may sit outside the box it was given —
                // the squeezed panel's neighbours are somebody else's
                // pixels.
                for line in &lines {
                    assert!(line.y >= area.y);
                }
                // And no line is ever empty: an empty run is a text call
                // that draws nothing at a cost.
                assert!(lines.iter().all(|line| !line.text.is_empty()));
            }
        }
        // The empty message is the degenerate input the wrap itself has
        // to survive, and a theme with no leading at all is the other.
        assert!(lay_out(Rect::new(0.0, 0.0, 100.0, 100.0), &looks[0], "", width_of).is_empty());
        assert!(!lay_out(Rect::new(0.0, 0.0, 1.0, 1e6), &looks[0], MESSAGE, |_| f32::NAN)
            .is_empty());
    }

    /// The empty state is READ from the theme rather than written here,
    /// and this is what that means when the theme says nothing: a raw
    /// look draws no message at all. A widget carrying a size or a
    /// colour of its own would still put one on the screen.
    #[test]
    fn a_theme_that_declares_nothing_draws_nothing() {
        assert!(!legible(&EmptyLook::raw()));
        // Either half is enough to silence it: type of no size cannot be
        // read, and ink of no alpha is not there.
        assert!(!legible(&look(0.4, 0.0, 1.2)));
        let mut invisible = look(0.4, 16.0, 1.2);
        invisible.ink = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 0.0 };
        assert!(!legible(&invisible));
        // And with both, it is drawn.
        assert!(legible(&look(0.4, 16.0, 1.2)));
    }

    /// `emptystate.role` is followed by NAME, so the names it makes are
    /// worth pinning: this is the half of the binding that lives in this
    /// crate, and `token_tests` checks the other half against the
    /// master.
    #[test]
    fn the_role_binding_names_the_roles_own_family() {
        assert_eq!(role_token("value", "size").as_deref(), Some("type.value.size"));
        assert_eq!(role_token("label.section", "fg").as_deref(), Some("type.label.section.fg"));
        // A master that binds no role is not given one here.
        assert_eq!(role_token("", "size"), None);
    }

    #[test]
    fn the_message_says_what_is_missing_and_promises_no_date() {
        // The wording is the widget's whole user interface, so it is
        // held to what the panel can actually answer for: the name of
        // the thing that is not there, and no calendar.
        assert!(MESSAGE.contains("nacelle-ai"));
        for promise in ["soon", "coming", "shortly", "will be", "next release"] {
            assert!(!MESSAGE.to_lowercase().contains(promise), "{promise:?} is a date");
        }
    }
}
