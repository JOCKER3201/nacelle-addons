//! The control panel, as a compiled widget.
//!
//! This is the worked example of the plugin boundary. The panel itself
//! is small — two unlabelled buttons — so what it demonstrates is the
//! shape rather than the difficulty: how a plugin attaches to the host,
//! what it draws through, and how it asks the application to act.
//!
//! Everything it draws goes through the host's function table. It never
//! sees a `Ctx`; it only passes the opaque handle back.
//!
//! Since ABI 5 the theme crosses that table as tokens. This file keeps
//! no colour, no length and no duration of its own: it resolves the
//! names it draws with once per theme epoch, asks the `button` row of
//! the class x state matrix for its fills, edges and labels, and when
//! the host cannot be asked — a master older than the tokens — it
//! degrades to the engine's raw look, never to the numbers that used
//! to be the design.

use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_EXIT,
    ACTION_NONE, ACTION_OPEN_SETTINGS, CORNER_CHAMFER, CORNER_ROUND, CORNER_SQUARE,
};
use std::borrow::Cow;
use std::ffi::c_void;
use std::time::Instant;

const BTN_EXIT: usize = 0;
const BTN_SETTINGS: usize = 1;
/// The buttons' names, as given. The case they draw in is
/// `type.button.case`'s decision, not the source string's.
const LABELS: [&str; 2] = ["Exit", "Settings"];

// The interaction states this widget can occupy, as indices into the
// matrix's declaration order (idle, hover, press, selected,
// selected_hover, dragging, disabled).
const STATE_IDLE: u32 = 0;
const STATE_HOVER: u32 = 1;
const STATE_PRESS: u32 = 2;

// Enum word indices, in each token's declared order — `default.theme`
// is the schema. `control.button.align = top | middle | bottom`;
// `type.button.case = none | upper | lower | smallcaps`.
const ALIGN_TOP: u32 = 0;
const ALIGN_MIDDLE: u32 = 1;
const CASE_UPPER: u32 = 1;
const CASE_LOWER: u32 = 2;
const CASE_SMALLCAPS: u32 = 3;

/// The host's interface, kept from the attach call. A plugin is loaded
/// once and never unloaded, so a static is the honest shape here.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    // Written once during attach, before any other call, and never
    // again; every read afterwards is on the frame thread.
    unsafe { HOST }
}

/// Whether the attached host carries the ABI 5 theme entries at all.
/// The static link always does, and `runtime::attach` refuses an older
/// dlopen master — but the check is what makes reading past the end of
/// an old table impossible rather than merely unlikely.
fn abi5(api: &HostApi) -> bool {
    api.abi_version >= 5
}

// The engine's raw answers, kept for a host too old to be asked. These
// mirror the defaults the ABI itself gives for a missing token — mid
// grey ink, no fill, one hairline — so the degrade is the same in both
// directions: undesigned, legible, and nobody's palette.
const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
const RAW_NONE: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
const RAW_STATE: StateStyleC = StateStyleC {
    fill: RAW_NONE,
    edge: RAW_INK,
    text: RAW_INK,
    glyph: RAW_INK,
    edge_width: 1.0,
    glow_radius: 0.0,
    glow_alpha: 0.0,
    elevation: 0.0,
};

/// A baked scalar — device px, milliseconds, a 0..1 fraction — or the
/// raw 0.0 when there is no token or no one to ask.
fn t_px(api: &HostApi, ctx: *mut c_void, id: u32) -> f32 {
    if abi5(api) {
        (api.theme_px)(ctx, id)
    } else {
        0.0
    }
}

/// The index of a token's word in its declared enum list, or the raw 0.
/// The word an enum token resolves to right now. The host owns the
/// vocabulary — an interned index means nothing across this edge — so
/// the plugin asks for the WORD and matches on it.
fn t_word(api: &HostApi, ctx: *mut c_void, id: u32) -> String {
    if !api.has_theme_enum_word() {
        return String::new();
    }
    let mut buf = [0u8; 32];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
}

fn t_enum(api: &HostApi, ctx: *mut c_void, id: u32) -> u32 {
    if abi5(api) {
        (api.theme_enum)(ctx, id)
    } else {
        0
    }
}

/// A flag, or the raw `false` when the host cannot be asked.
fn t_flag(api: &HostApi, ctx: *mut c_void, id: u32) -> bool {
    abi5(api) && (api.theme_flag)(ctx, id) != 0
}

/// A colour used as ink, or the raw grey when the host cannot be asked.
fn t_col(api: &HostApi, ctx: *mut c_void, id: u32) -> ColorC {
    if abi5(api) {
        (api.theme_color)(ctx, id)
    } else {
        RAW_INK
    }
}

/// One rung of the state ladder for one class, or the raw rung when the
/// host cannot be asked or answers nothing.
fn t_state(api: &HostApi, ctx: *mut c_void, class: u32, state: u32) -> StateStyleC {
    if !abi5(api) {
        return RAW_STATE;
    }
    let mut out = RAW_STATE;
    let n = (api.theme_class_state)(
        ctx,
        class,
        state,
        &mut out,
        std::mem::size_of::<StateStyleC>() as u32,
    );
    if n == 0 {
        RAW_STATE
    } else {
        out
    }
}

/// The token ids this widget reads. Names are stable; ids are
/// per-master-load, so the whole set is resolved again whenever
/// `theme_epoch` moves — a theme swap, a mood, a resize.
#[derive(Clone, Copy)]
struct ThemeIds {
    epoch: u32,
    button_h: u32,
    button_gap: u32,
    button_w_frac: u32,
    button_align: u32,
    skew: u32,
    corner: u32,
    corner_style: u32,
    type_size: u32,
    type_min_px: u32,
    type_leading: u32,
    type_tracking: u32,
    type_case: u32,
    press_ms: u32,
    /// The glyph slot beside the caption (u2 §2.12): image 9's taskbar
    /// buttons, a glyph leading a caption. The glyph is decoration and
    /// the caption survives verbatim; icon_size is floored by
    /// button.icon_size_min_px at bake, so no floor lives here.
    icon_size: u32,
    icon_gap: u32,
    icon_stroke: u32,
    /// Whether the destructive caption's button takes severity.critical
    /// — image 5's SYSTEM LOCKDOWN, the THEME's decision (u2 §2.12).
    emphasis: u32,
    crit_edge: u32,
    crit_text: u32,
    crit_glyph: u32,
    /// The `button` row of the class x state matrix — a class index,
    /// not a token id.
    class_button: u32,
}

fn resolve_ids(api: &HostApi, epoch: u32) -> ThemeIds {
    let tok = |name: &str| {
        if abi5(api) {
            (api.theme_token)(name.as_ptr(), name.len() as u32)
        } else {
            u32::MAX
        }
    };
    ThemeIds {
        epoch,
        button_h: tok("control.button.h"),
        button_gap: tok("control.button.gap"),
        button_w_frac: tok("control.button.w_frac"),
        button_align: tok("control.button.align"),
        // The same shear every parallelogram button reads, so the two
        // objects called "button" finally agree on a shape.
        skew: tok("button.skew"),
        corner: tok("button.corner"),
        corner_style: tok("button.corner_style"),
        type_size: tok("type.button.size"),
        type_min_px: tok("type.button.min_px"),
        type_leading: tok("type.button.leading"),
        type_tracking: tok("type.button.tracking"),
        type_case: tok("type.button.case"),
        press_ms: tok("motion.press.duration_ms"),
        icon_size: tok("button.icon_size"),
        icon_gap: tok("button.icon_gap"),
        icon_stroke: tok("icon.stroke"),
        emphasis: tok("button.emphasis_from_severity"),
        crit_edge: tok("severity.critical.edge"),
        crit_text: tok("severity.critical.text"),
        crit_glyph: tok("severity.critical.glyph"),
        class_button: if abi5(api) {
            (api.theme_class)("button".as_ptr(), "button".len() as u32)
        } else {
            u32::MAX
        },
    }
}

/// The widget's own state: which button was pressed and when — WHICH
/// state each button is in is this file's to remember; how long the
/// press flash lasts is `motion.press.duration_ms`'s to say — plus the
/// resolved token ids, cached per epoch.
struct Control {
    pressed: [Option<Instant>; 2],
    theme: Option<ThemeIds>,
}

/// The cached ids, re-resolved when the epoch has moved.
fn theme(this: &mut Control, api: &HostApi, ctx: *mut c_void) -> ThemeIds {
    let epoch = if abi5(api) { (api.theme_epoch)(ctx) } else { 0 };
    match this.theme {
        Some(t) if t.epoch == epoch => t,
        _ => {
            let t = resolve_ids(api, epoch);
            this.theme = Some(t);
            t
        }
    }
}

/// Button rectangles, stacked in the panel. Height, gap and width
/// fraction are the `[control]` group's numbers; where the stack sits
/// is `control.button.align`'s word. Nothing here reads the window:
/// the height is a token, so the controls stay the same size wherever
/// the panel is put.
fn button_rects(h: f32, gap: f32, w_frac: f32, align: u32, r: RectC) -> [RectC; 2] {
    let w = r.w * w_frac;
    let x = r.x + (r.w - w) / 2.0;
    let stack = h * 2.0 + gap;
    let top = match align {
        ALIGN_TOP => r.y + gap,
        ALIGN_MIDDLE => r.y + (r.h - stack) / 2.0,
        _ => r.y + r.h - gap - stack,
    };
    [
        RectC { x, y: top, w, h },
        RectC { x, y: top + h + gap, w, h },
    ]
}

/// The label in the case `type.button.case` asks for. Smallcaps needs
/// the per-glyph sizes only the host's font system has; through a
/// single text call the nearest honest reading is capitals.
fn cased(s: &str, case: u32) -> Cow<'_, str> {
    match case {
        CASE_UPPER | CASE_SMALLCAPS => Cow::Owned(s.to_uppercase()),
        CASE_LOWER => Cow::Owned(s.to_lowercase()),
        _ => Cow::Borrowed(s),
    }
}

/// Two buttons and the gaps around them — the whole of this widget, so
/// the box around it is exactly that tall.
extern "C" fn sizing(instance: *mut c_void, ctx: *mut c_void, _: *const c_void) -> f32 {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return -1.0 };
    let t = theme(this, api, ctx);
    let h = t_px(api, ctx, t.button_h);
    let gap = t_px(api, ctx, t.button_gap);
    h * 2.0 + gap * 2.0
}

fn contains(r: &RectC, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Control { pressed: [None, None], theme: None })) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Control) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Control> {
    unsafe { (instance as *mut Control).as_mut() }
}

extern "C" fn draw(
    instance: *mut c_void,
    ctx: *mut c_void,
    _host: *const c_void,
    r: RectC,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    let t = theme(this, api, ctx);

    let h = t_px(api, ctx, t.button_h);
    let gap = t_px(api, ctx, t.button_gap);
    let w_frac = t_px(api, ctx, t.button_w_frac);
    let align = t_enum(api, ctx, t.button_align);
    let rects = button_rects(h, gap, w_frac, align, r);

    let skew = t_px(api, ctx, t.skew);
    let px = t_px(api, ctx, t.type_size).max(t_px(api, ctx, t.type_min_px));
    let leading = t_px(api, ctx, t.type_leading);
    let tracking = px * t_px(api, ctx, t.type_tracking);
    let case = t_enum(api, ctx, t.type_case);
    let press_ms = t_px(api, ctx, t.press_ms);

    let (mut mx, mut my) = (0.0f32, 0.0f32);
    (api.mouse)(ctx, &mut mx, &mut my);

    for (i, br) in rects.iter().enumerate() {
        let hover = contains(br, mx, my);
        // A decaying click flash IS the press state; only the moment of
        // the click is this widget's to keep.
        let flash = this.pressed[i]
            .map(|p| p.elapsed().as_secs_f32() * 1000.0 < press_ms)
            .unwrap_or(false);
        let state = if flash {
            STATE_PRESS
        } else if hover {
            STATE_HOVER
        } else {
            STATE_IDLE
        };
        let mut style = t_state(api, ctx, t.class_button, state);
        // The destructive caption's emphasis — image 5's LOCKDOWN button
        // — is the THEME's decision through button.emphasis_from_severity
        // (u2 §2.12), never this plugin's: which button is destructive is
        // this side's judgement of the ACTION, the colours are the
        // severity's own, and the master ships the flag off.
        if i == BTN_EXIT && t_flag(api, ctx, t.emphasis) {
            style.edge = t_col(api, ctx, t.crit_edge);
            style.text = t_col(api, ctx, t.crit_text);
            style.glyph = t_col(api, ctx, t.crit_glyph);
        }
        // The same shape as every other button in the interface, which
        // since the corners moved into the theme means the shape of the
        // FRAMES around it. A host old enough to lack the ring pair
        // draws the flat quad it always did — visibly plainer, never
        // wrong.
        let rc = RectC { x: br.x, y: br.y, w: br.w, h: br.h };
        if api.has_ring() && skew <= 0.0 {
            let radius = t_px(api, ctx, t.corner);
            let cs = match t_word(api, ctx, t.corner_style).as_str() {
                "round" => CORNER_ROUND,
                "chamfer" => CORNER_CHAMFER,
                _ => CORNER_SQUARE,
            };
            (api.ring_fill)(ctx, rc, cs, radius, style.fill);
            if style.edge_width > 0.0 {
                (api.ring)(ctx, rc, cs, radius, style.edge_width, style.edge);
            }
        } else {
            let pts = [
                br.x + skew, br.y,
                br.x + br.w, br.y,
                br.x + br.w - skew, br.y + br.h,
                br.x, br.y + br.h,
            ];
            (api.quad)(ctx, pts.as_ptr(), style.fill);
            if style.edge_width > 0.0 {
                (api.polyline)(ctx, pts.as_ptr(), 4, style.edge_width, style.edge, true);
            }
        }

        let label = cased(LABELS[i], case);
        let bytes = label.as_bytes();
        // The glyph slot (u2 §2.12): a glyph leading the caption, the two
        // centred as one group — image 9's taskbar button. The glyphs are
        // the icon registry's `power` and `gear` slots' BUILT-IN FALLBACK
        // (icon.fallback = builtin): the engine does not bake
        // icon.<name>.layers across the ABI yet, so the compiled-in
        // recipe is the drawn form, in the rung's glyph colour, at
        // button.icon_size. The caption is the content and is unchanged.
        let s = t_px(api, ctx, t.icon_size);
        if s > 0.0 {
            let gap_i = t_px(api, ctx, t.icon_gap);
            let stroke = t_px(api, ctx, t.icon_stroke).max(1.0);
            let tw = (api.measure)(
                ctx,
                0, // the interface font
                px,
                bytes.as_ptr(),
                bytes.len() as u32,
                tracking,
            );
            let group = 2.0 * s + gap_i + tw;
            let left = br.x + (br.w - group) / 2.0;
            let cx = left + s;
            let cy = br.y + br.h / 2.0;
            if i == BTN_EXIT {
                draw_power(api, ctx, cx, cy, s, stroke, style.glyph);
            } else {
                draw_gear(api, ctx, cx, cy, s, stroke, style.glyph);
            }
            (api.text)(
                ctx,
                0, // the interface font
                px,
                left + 2.0 * s + gap_i,
                br.y + (br.h - px * leading) / 2.0,
                bytes.as_ptr(),
                bytes.len() as u32,
                style.text,
                tracking,
                0, // from the glyph's right edge
            );
        } else {
            // No glyph slot: the caption alone, centred, as it ever was.
            (api.text)(
                ctx,
                0, // the interface font
                px,
                br.x + br.w / 2.0,
                br.y + (br.h - px * leading) / 2.0,
                bytes.as_ptr(),
                bytes.len() as u32,
                style.text,
                tracking,
                1, // centred
            );
        }
    }
}

/// The power mark — an arc opened at the top and a stem through the
/// opening. The proportions are the glyph itself, what a filled
/// icon.power.layers replaces wholesale, not what a theme tunes.
fn draw_power(
    api: &HostApi,
    ctx: *mut c_void,
    cx: f32,
    cy: f32,
    s: f32,
    stroke: f32,
    c: ColorC,
) {
    const SEGS: usize = 14;
    let start = (-90.0_f32 + 35.0).to_radians();
    let sweep = 290.0_f32.to_radians();
    let mut pts = [0.0f32; (SEGS + 1) * 2];
    for (k, p) in pts.chunks_exact_mut(2).enumerate() {
        let a = start + sweep * k as f32 / SEGS as f32;
        p[0] = cx + s * a.cos();
        p[1] = cy + s * a.sin();
    }
    (api.polyline)(ctx, pts.as_ptr(), (SEGS + 1) as u32, stroke, c, false);
    (api.line)(ctx, cx, cy - s * 1.15, cx, cy - s * 0.2, stroke, c);
}

/// The gear — an eight-toothed ring around a hub. Same rule: the
/// proportions are the glyph's own, replaced whole by icon.gear.layers.
fn draw_gear(
    api: &HostApi,
    ctx: *mut c_void,
    cx: f32,
    cy: f32,
    s: f32,
    stroke: f32,
    c: ColorC,
) {
    const TEETH: usize = 8;
    let inner = s * 0.72;
    let mut pts = [0.0f32; TEETH * 4];
    for (k, p) in pts.chunks_exact_mut(2).enumerate() {
        let r = if k % 2 == 0 { s } else { inner };
        let a = std::f32::consts::PI * k as f32 / TEETH as f32;
        p[0] = cx + r * a.cos();
        p[1] = cy + r * a.sin();
    }
    (api.polyline)(ctx, pts.as_ptr(), (TEETH * 2) as u32, stroke, c, true);
    const HUB: usize = 10;
    let hub_r = s * 0.35;
    let mut hub = [0.0f32; HUB * 2];
    for (k, p) in hub.chunks_exact_mut(2).enumerate() {
        let a = std::f32::consts::TAU * k as f32 / HUB as f32;
        p[0] = cx + hub_r * a.cos();
        p[1] = cy + hub_r * a.sin();
    }
    (api.polyline)(ctx, hub.as_ptr(), HUB as u32, stroke, c, true);
}

extern "C" fn click(
    instance: *mut c_void,
    x: f32,
    y: f32,
    r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    let (Some(api), Some(this), Some(out)) =
        (host(), state(instance), unsafe { out.as_mut() })
    else {
        return;
    };
    // Input arrives outside a frame, so there is no drawing context to
    // pass. The theme entries never read it — the parameter is room for
    // a future per-window theme — and a null one is what "no frame"
    // honestly is.
    let ctx = std::ptr::null_mut();
    let t = theme(this, api, ctx);
    let h = t_px(api, ctx, t.button_h);
    let gap = t_px(api, ctx, t.button_gap);
    let w_frac = t_px(api, ctx, t.button_w_frac);
    let align = t_enum(api, ctx, t.button_align);
    for (i, br) in button_rects(h, gap, w_frac, align, r).iter().enumerate() {
        if contains(br, x, y) {
            this.pressed[i] = Some(Instant::now());
            out.kind = match i {
                BTN_EXIT => ACTION_EXIT,
                BTN_SETTINGS => ACTION_OPEN_SETTINGS,
                _ => ACTION_NONE,
            };
            return;
        }
    }
}

extern "C" fn wheel(_: *mut c_void, _: f32, _: RectC, _: f32, _: f32, _: *mut ActionC) {}

extern "C" fn grid(_: *mut c_void, _: *mut u32, _: *mut u32) {}

extern "C" fn key_feedback(_: *mut c_void, _: u32, _: *const u8, _: u32) {}

/// No title band: the control panel shows no heading today, and a band
/// would grow the panel's intrinsic height at `processes`' expense
/// (u2 §4.3's recommendation) — the theme opts it in later, not this
/// plugin.
extern "C" fn chrome(
    _: *mut c_void,
    _: *mut c_void,
    _: *const c_void,
    _: *mut ChromeC,
    _: u32,
) -> u32 {
    0
}

/// This widget takes no drags: declining every Begin keeps a press on
/// the ordinary click path.
#[allow(clippy::too_many_arguments)]
extern "C" fn drag(
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
    draw,
    click,
    wheel,
    grid,
    key_feedback,
    sizing,
    chrome,
    drag,
};

/// The attach point the host looks for. Taking the host's interface here
/// — rather than reaching for the toolkit's own statics — is what stops
/// this library from quietly keeping a second copy of state that is
/// supposed to exist once.
///
/// # Safety
/// Called by the host with its own interface, once, before anything else.
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

#[cfg(feature = "dyn")]
#[no_mangle]
pub unsafe extern "C" fn nacelle_plugin_attach(api: *const HostApi) -> *const PluginApi {
    if !nacelle::runtime::attach(api) {
        // Versions disagree: say nothing more and let the host skip us.
        return std::ptr::null();
    }
    HOST = api.as_ref();
    &API
}
