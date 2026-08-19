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

use nacelle::ui::Case;
use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_EXIT,
    ACTION_NONE, ACTION_OPEN_SETTINGS, CORNER_CHAMFER, CORNER_ROUND, CORNER_SQUARE,
};
use nacelle::widget::factory::BuiltinWidget;
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
// is the schema. `control.button.align = top | middle | bottom`.
//
// `type.button.case` used to be numbered here too, and is not any more:
// an index only names a word against the schema it was interned in, and
// this side has no schema. The word crosses through `theme_enum_word`
// and the toolkit's own `ui::Case` reads it, so a master that reorders
// its list — or a theme that misspells the word — answers the same here
// as it does in the panel band.
const ALIGN_TOP: u32 = 0;
const ALIGN_MIDDLE: u32 = 1;

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

// What a host too old to be asked draws with: nothing at all.
//
// Not a grey. A colour chosen where the theme cannot be reached is a
// design decision made in the dark, and this program has none of those —
// so the panel degrades to no ink and no width, and the buttons simply
// do not appear. The same clean bail `ai` takes for the same host.
const NO_INK: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
const RAW_STATE: StateStyleC = StateStyleC {
    fill: NO_INK,
    edge: NO_INK,
    text: NO_INK,
    glyph: NO_INK,
    edge_width: 0.0,
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
        NO_INK
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
    button_fill: u32,
    // type.<button.role>.* — the role the master BINDS a button's
    // caption to. It named `button` and this file spelled `button` out,
    // which is the binding written twice: only one of the two moves
    // when a theme re-roles its controls.
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

fn resolve_ids(api: &HostApi, ctx: *mut c_void, epoch: u32) -> ThemeIds {
    let tok = |name: &str| {
        if abi5(api) {
            (api.theme_token)(name.as_ptr(), name.len() as u32)
        } else {
            u32::MAX
        }
    };
    // The caption's binding, followed to the role it names. A master
    // that binds no role leaves every id below MISSING — a caption of
    // no size, drawn as nothing, rather than one this file sized.
    let role = t_word(api, ctx, tok("button.role"));
    let of = |suffix: &str| {
        if role.is_empty() {
            u32::MAX
        } else {
            tok(&format!("type.{role}.{suffix}"))
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
        button_fill: tok("component.button.fill"),
        type_size: of("size"),
        type_min_px: of("min_px"),
        type_leading: of("leading"),
        type_tracking: of("tracking"),
        type_case: of("case"),
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

/// How many drawn content boxes this widget remembers at once.
///
/// One per screen would do — the desktop draws the SAME instance once per
/// screen, each with its own content box — but the count is not knowable
/// from here, so four covers any desk this program has met with room to
/// spare. A fifth concurrent box evicts the oldest entry, which only
/// costs that box the fallback path until its next frame writes it back.
const DRAWN_SLOTS: usize = 4;

/// One screen's frame: the content box the host handed `draw`, and the
/// buttons the frame put there.
#[derive(Clone, Copy)]
struct Drawn {
    r: RectC,
    rects: [RectC; 2],
    /// When the entry was written, on the instance's own draw counter —
    /// what "oldest" means when every slot is taken.
    stamp: u64,
}

/// Exact equality, not a tolerance: both boxes come from the same host
/// field, so any difference is a real layout change and never rounding.
/// A tolerance here would keep answering with rectangles from a layout
/// that has moved.
fn same_box(a: RectC, b: RectC) -> bool {
    a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h
}

/// The widget's own state: which button was pressed and when — WHICH
/// state each button is in is this file's to remember; how long the
/// press flash lasts is `motion.press.duration_ms`'s to say — plus the
/// resolved token ids, cached per epoch.
struct Control {
    pressed: [Option<Instant>; 2],
    theme: Option<ThemeIds>,
    /// The content boxes recent frames drew in, and the buttons each
    /// frame put there — one entry per box, because ONE slot was itself
    /// the bug. The desktop draws this single instance once per screen
    /// with a different content box each time, so screen B's frame used
    /// to overwrite screen A's rectangles: input from A missed the store,
    /// fell to the fallback, and measured with whichever screen last
    /// published a bake. Input is answered against THESE entries, never
    /// against a fresh calculation.
    ///
    /// WHY the fresh calculation is wrong, and it is not caution:
    /// `button_rects` reads its height and gap from theme tokens, the
    /// tokens are sized in `u`, and `u` comes from the window height of
    /// whichever screen last published a bake. In a frame that is this
    /// screen — `draw_screen` sets the viewport first. Outside a frame it
    /// is whoever drew last, so on a desktop of unequal monitor heights
    /// the same call answers with the OTHER screen's button size.
    /// Measured on 2560x1440 beside 3840x2160: the boundary between the
    /// two buttons lands 35.8 px away from the drawn one.
    ///
    /// The consequence is worse than a missed press. A click that lands in
    /// the gap between where SETTINGS is drawn and where it is tested hits
    /// EXIT instead, and EXIT closes the desktop.
    ///
    /// Storing what was drawn makes the question unanswerable-wrong: the
    /// rectangle a person can see IS the rectangle that answers. The real
    /// fix belongs in the toolkit — an object should record its own hit
    /// geometry and input should test the record, for every control rather
    /// than this one — and when that lands this field goes with it.
    drawn: [Option<Drawn>; DRAWN_SLOTS],
    /// The draw counter behind `Drawn::stamp`.
    frame: u64,
}

/// The buttons a frame drew in THIS box, whichever screen's frame it was.
///
/// Split out of `hit_rects` so the decision can be stated without a host:
/// everything that makes answering input correct is these comparisons over
/// the stored entries, and the fallback around it is the old path left
/// alone. Every entry is searched — the box IS the key, so screen A's
/// input finds screen A's buttons no matter which screen drew last.
fn stored_for(drawn: &[Option<Drawn>], r: RectC) -> Option<[RectC; 2]> {
    drawn.iter().flatten().find(|d| same_box(d.r, r)).map(|d| d.rects)
}

/// Records what a frame drew: its own box's entry replaced in place, a
/// free slot taken otherwise, the oldest entry evicted only when every
/// slot belongs to some other box. Each screen's frame maintains its own
/// entry and touches nobody else's — the overwriting that used to lose
/// screen A's rectangles to screen B's frame cannot be expressed here.
fn record_drawn(this: &mut Control, r: RectC, rects: [RectC; 2]) {
    this.frame += 1;
    let entry = Drawn { r, rects, stamp: this.frame };
    let slot = this
        .drawn
        .iter()
        .position(|e| e.map(|d| same_box(d.r, r)).unwrap_or(false))
        .or_else(|| this.drawn.iter().position(|e| e.is_none()))
        .unwrap_or_else(|| {
            this.drawn
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.map(|d| d.stamp).unwrap_or(0))
                .map(|(i, _)| i)
                .unwrap_or(0)
        });
    this.drawn[slot] = Some(entry);
}

/// The buttons to test an event against: the ones on screen.
///
/// Falls back to calculating them only when NO frame has drawn in this
/// content box yet — an instance before its first frame, or a layout change
/// with a redraw already on its way. A box another screen drew in is not
/// such a case any more: each screen's box keeps its own entry, so the
/// fallback — the old, viewport-dependent path — stays narrow on purpose.
fn hit_rects(this: &mut Control, api: &HostApi, r: RectC) -> [RectC; 2] {
    if let Some(rects) = stored_for(&this.drawn, r) {
        return rects;
    }
    let ctx = std::ptr::null_mut();
    let t = theme(this, api, ctx);
    button_rects(
        t_px(api, ctx, t.button_h),
        t_px(api, ctx, t.button_gap),
        t_px(api, ctx, t.button_w_frac),
        t_enum(api, ctx, t.button_align),
        r,
    )
}

/// The cached ids, re-resolved when the epoch has moved.
fn theme(this: &mut Control, api: &HostApi, ctx: *mut c_void) -> ThemeIds {
    let epoch = if abi5(api) { (api.theme_epoch)(ctx) } else { 0 };
    match this.theme {
        Some(t) if t.epoch == epoch => t,
        _ => {
            let t = resolve_ids(api, ctx, epoch);
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
    Box::into_raw(Box::new(Control {
        pressed: [None, None],
        theme: None,
        drawn: [None; DRAWN_SLOTS],
        frame: 0,
    })) as *mut c_void
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
    // Kept for input to answer against. Everything above ran under this
    // screen's viewport; nothing outside a frame can say the same. Written
    // into THIS box's entry: on a desktop of two screens the same instance
    // draws once per screen with a different box, and each frame must keep
    // its own record rather than overwrite the other screen's.
    record_drawn(this, r, rects);

    let skew = t_px(api, ctx, t.skew);
    let px = t_px(api, ctx, t.type_size).max(t_px(api, ctx, t.type_min_px));
    let leading = t_px(api, ctx, t.type_leading);
    let tracking = px * t_px(api, ctx, t.type_tracking);
    let case = Case::from_word(&t_word(api, ctx, t.type_case));
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
            // The OPAQUE plate first, as object::button lays it, so the
            // class's near-transparent idle wash rides a solid button and
            // not the panel's own bed — one button colour in every window
            // (JEDEN MODEL OKNA).
            (api.ring_fill)(ctx, rc, cs, radius, t_col(api, ctx, t.button_fill));
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
            (api.quad)(ctx, pts.as_ptr(), t_col(api, ctx, t.button_fill));
            (api.quad)(ctx, pts.as_ptr(), style.fill);
            if style.edge_width > 0.0 {
                (api.polyline)(ctx, pts.as_ptr(), 4, style.edge_width, style.edge, true);
            }
        }

        let label = nacelle::ui::recase(case, LABELS[i]);
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
            // `icon.stroke` is declared in em, and an em token bakes to
            // the BARE multiplier — `length_px` passes `Unit::Em`
            // through untouched, because the size it multiplies is not
            // known where the theme is baked. So `theme_px` answers
            // 0.10 here, and 0.10 is not a width: what states the em is
            // the size this slot draws its glyph at, which is the very
            // token read one line up. Handing the multiplier straight
            // to `polyline` drew a tenth-of-a-pixel hair; clamping it to
            // one device px, as this file did before, drew the widget's
            // number instead of the theme's. Neither read the master.
            let stroke = t_px(api, ctx, t.icon_stroke) * s;
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
    // pass — and the theme entries do not take one anyway. That used to
    // read as harmless here ("the parameter is room for a future per-window
    // theme"), which was wrong in a way that could close the desktop: they
    // read the ONE published bake, and outside a frame that bake belongs to
    // whichever screen drew last. So the buttons are not recalculated; the
    // ones on screen answer. See `Control::drawn`.
    for (i, br) in hit_rects(this, api, r).iter().enumerate() {
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

/// The two buttons are the only controls this widget has, so the hand
/// appears over them and nowhere else. The rectangles are the ones the
/// click uses, from the same tokens: the application asks rather than
/// working them out a second time and drifting from what was drawn.
extern "C" fn pointer(
    instance: *mut c_void,
    x: f32,
    y: f32,
    r: RectC,
    _win_w: f32,
    _win_h: f32,
) -> u32 {
    let (Some(api), Some(this)) = (host(), state(instance)) else {
        return 0;
    };
    // The buttons on screen, for the same reason as `click` above: a fresh
    // calculation here would size them from another screen's bake, and the
    // cursor would turn into a hand over a strip where nothing is drawn.
    let over = hit_rects(this, api, r).iter().any(|br| contains(br, x, y));
    u32::from(over)
}

/// Filled, and consumes nothing on purpose: the two buttons are reached
/// with the pointer, and this panel keeps no idea of which of them the
/// keyboard is on. Answering 0 leaves Tab and the arrows to the host's
/// focus chain, which is the thing that would have to walk them.
extern "C" fn key(
    _: *mut c_void,
    _: u32,
    _: *const u8,
    _: u32,
    _: u32,
    _: *mut ActionC,
) -> u32 {
    0
}

/// Filled, and does nothing on purpose. The press rung this entry
/// carries is one this panel already draws from its own clock — a button
/// marks itself for `motion.press.duration_ms` from the click — so
/// taking the press here as well would be a second source of one state,
/// and the two would disagree the first time a press was released off
/// the button it started on.
#[allow(clippy::too_many_arguments)]
extern "C" fn button(
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
    pointer,
    key,
    button,
};

/// This addon, for a host that LINKS the crate in instead of loading
/// `control.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `control.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "control",
    meta: include_str!("../control.meta"),
    attach: builtin_attach,
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

#[cfg(test)]
mod abi_tests {
    use super::*;
    use nacelle::runtime::{
        BUTTON_PRESS, BUTTON_RELEASE, MODS_CTRL, PLUGIN_API_HAS_BUTTON,
    };

    /// A value no entry of this widget could ever write, so "left alone"
    /// is something a test can see.
    fn untouched() -> ActionC {
        ActionC { kind: u32::MAX, index: 0, lines: 0, data: std::ptr::null(), data_len: 0 }
    }

    /// The entries appended in this version are filled AND declared.
    ///
    /// Two different things, and the host checks the second: it reads
    /// `api_size` before it calls either, so a table that carried the
    /// pointers without reaching them would be a widget the host never
    /// asks. `size_of` says it here because the table is one literal.
    ///
    /// That they do NOTHING is pinned too. It is a decision, written out
    /// above `key` and `button`, and a later change that gave
    /// this widget a keyboard or a press rung has to come past those
    /// reasons — and past this test — rather than round them.
    #[test]
    fn the_appended_entries_are_declared_and_take_nothing() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);

        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut a = untouched();
        // A character, a named key and a chord: nothing here is the
        // widget's, so the host keeps all three.
        assert_eq!(
            (API.key)(std::ptr::null_mut(), 'a' as u32, std::ptr::null(), 0, 0, &mut a),
            0
        );
        let word = nacelle::runtime::keys::DOWN;
        assert_eq!(
            (API.key)(std::ptr::null_mut(), 0, word.as_ptr(), word.len() as u32, 0, &mut a),
            0
        );
        assert_eq!(
            (API.key)(std::ptr::null_mut(), 'c' as u32, std::ptr::null(), 0, MODS_CTRL, &mut a),
            0
        );
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(std::ptr::null_mut(), phase, 1.0, 1.0, r, 100.0, 100.0, &mut a);
        }
        assert_eq!(a.kind, u32::MAX, "an entry that does nothing writes nothing");
    }
}

#[cfg(test)]
mod stroke_tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static STROKES: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    extern "C" fn rec_polyline(
        _: *mut c_void,
        _: *const f32,
        _: u32,
        t: f32,
        _: ColorC,
        _: bool,
    ) {
        STROKES.with(|s| s.borrow_mut().push(t));
    }

    extern "C" fn rec_line(_: *mut c_void, _: f32, _: f32, _: f32, _: f32, t: f32, _: ColorC) {
        STROKES.with(|s| s.borrow_mut().push(t));
    }

    /// Off every button, so no rung but idle is reached and the frame is
    /// the resting one.
    extern "C" fn no_mouse(_: *mut c_void, x: *mut f32, y: *mut f32) {
        unsafe {
            *x = f32::NAN;
            *y = f32::NAN;
        }
    }

    fn px(name: &str) -> f32 {
        nacelle::theme::resolved().px(nacelle::theme::id(name).expect(name))
    }

    /// Every stroke width the two glyphs leave, taken through the real
    /// entry: a host table whose stroking entries write the widths down
    /// and whose theme half is the loaded master's own.
    fn widths() -> Vec<f32> {
        nacelle::theme::load();
        let api: &'static HostApi = Box::leak(Box::new(HostApi {
            polyline: rec_polyline,
            line: rec_line,
            mouse: no_mouse,
            ..*nacelle::plugin::host_api()
        }));
        builtin_attach(api);
        let instance = create();
        STROKES.with(|s| s.borrow_mut().clear());
        // A null drawing context: every theme entry ignores it and the
        // stroking entries here are ours, so the widths are real and the
        // rasterising is not attempted.
        draw(instance, std::ptr::null_mut(), std::ptr::null(), RectC {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        });
        destroy(instance);
        STROKES.with(|s| s.borrow().clone())
    }

    /// `icon.stroke` is an em, and an em BAKES TO THE BARE MULTIPLIER —
    /// `length_px` has nothing to multiply it by where the theme is
    /// baked, so the number the ABI answers with (0.10) is a ratio and
    /// not a width. This pins the size that states it: the glyph's own,
    /// `button.icon_size`, the token `draw` reads one line above the
    /// stroke. Both mistakes this file has made are failures here — the
    /// multiplier handed over raw (0.10 px, an invisible hair) and the
    /// old `.max(1.0)` (the widget's number in place of the theme's).
    #[test]
    fn the_glyph_stroke_is_the_masters_em_of_the_size_the_glyph_draws_at() {
        // `button.icon_size` arrives already floored by its
        // `_min_px` companion — bake bounds X by X_min_px as a rule —
        // so this is the same one number `draw` sizes the glyph with.
        let want = px("icon.stroke") * px("button.icon_size");
        let widths = widths();
        assert!(!widths.is_empty(), "neither glyph was stroked");
        for w in &widths {
            assert_eq!(*w, want, "a glyph stroke of {w} where the master states {want}");
        }
        // The two numbers this file used to draw instead, named so a
        // return to either fails rather than passes quietly.
        assert!(want > px("icon.stroke"), "the em multiplier is being drawn as a width");
        assert!(want != 1.0, "the retired floor is back");
    }

    /// The floor is the master's to state, and it does not state one:
    /// the X / X_min_px convention (button.icon_size_min_px is one) has
    /// no entry for this stroke, so nothing here rounds it up. When the
    /// key appears, it is read here — and this is what says so.
    #[test]
    fn there_is_still_no_companion_floor_for_this_stroke_to_read() {
        nacelle::theme::load();
        assert!(
            nacelle::theme::id("icon.stroke_min_px").is_none(),
            "icon.stroke_min_px exists now — read it in `draw` instead of nothing"
        );
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;

    /// The owner's desktop, in the two unit sizes it produces.
    ///
    /// `u = clamp(window_h * 0.005, 4, 10)`, so a 2160-line screen sits on
    /// the 10 px ceiling and a 1440-line one lands at 7.2. `button.h` is
    /// 8.4u and `button.gap` is 0.35 of that, which is where these four
    /// numbers come from — written out rather than derived, so the test
    /// keeps stating the case even if the tokens move.
    const TALL_H: f32 = 84.0;
    const TALL_GAP: f32 = 29.4;
    const SHORT_H: f32 = 60.48;
    const SHORT_GAP: f32 = 21.168;
    const W_FRAC: f32 = 0.8;
    const ALIGN_BOTTOM: u32 = 2;

    fn box_of(h: f32) -> RectC {
        RectC { x: 0.0, y: 0.0, w: 300.0, h }
    }

    /// A fresh instance, as `create` builds one, without going through the
    /// pointer dance — these tests only exercise the drawn-entry store.
    fn fresh() -> Control {
        Control { pressed: [None, None], theme: None, drawn: [None; DRAWN_SLOTS], frame: 0 }
    }

    /// Input answers with the buttons a person can SEE, not with buttons
    /// measured again after the frame ended.
    ///
    /// The two calculations differ because `button_rects` sizes itself from
    /// theme tokens, the tokens are scaled by the window height of whichever
    /// screen last published a bake, and input runs outside a frame — so on
    /// a desktop of unequal monitors it measures with the other screen's
    /// numbers. This test pins the fix by showing the two answers ARE
    /// different and that the drawn one is the one returned.
    #[test]
    fn input_answers_with_the_buttons_that_were_drawn() {
        let r = box_of(TALL_H * 2.0 + TALL_GAP);
        let drawn = button_rects(TALL_H, TALL_GAP, W_FRAC, ALIGN_BOTTOM, r);
        let measured_again = button_rects(SHORT_H, SHORT_GAP, W_FRAC, ALIGN_BOTTOM, r);

        // If these ever agree the test proves nothing, so it says so first.
        let drawn_edge = drawn[1].y;
        let stale_edge = measured_again[1].y;
        assert!(
            (drawn_edge - stale_edge).abs() > 1.0,
            "the two screens produced the same buttons ({drawn_edge} vs {stale_edge}); \
             this test can no longer tell the fix from the bug"
        );

        let mut this = fresh();
        record_drawn(&mut this, r, drawn);
        let answered = stored_for(&this.drawn, r).expect("the drawn box was not recognised");
        assert_eq!(
            answered[1].y, drawn_edge,
            "input answered with buttons that were never on screen"
        );
    }

    /// A moved panel falls back rather than answering from the old place.
    ///
    /// The stored rectangles belong to the box they were drawn in. When the
    /// host hands input a box NO frame has drawn in, the layout has changed
    /// and a redraw is already coming; answering with a previous frame's
    /// rectangles would put the buttons where the panel no longer is.
    #[test]
    fn a_different_content_box_is_not_answered_from_the_last_frame() {
        let r = box_of(TALL_H * 2.0 + TALL_GAP);
        let drawn = button_rects(TALL_H, TALL_GAP, W_FRAC, ALIGN_BOTTOM, r);
        let mut this = fresh();
        record_drawn(&mut this, r, drawn);
        let moved = RectC { y: r.y + 40.0, ..r };
        assert!(
            stored_for(&this.drawn, moved).is_none(),
            "a panel that moved was still answered from where it used to be"
        );
        assert!(
            stored_for(&fresh().drawn, r).is_none(),
            "an instance that has never drawn claimed to know its buttons"
        );
    }

    /// The two-monitor frame, the case the single slot used to lose.
    ///
    /// The desktop draws the SAME instance once per screen, each screen
    /// with its own content box and its own button sizes. With one slot,
    /// screen B's frame overwrote screen A's record, so input from A found
    /// nothing stored and fell back to the miscalculating path — the loop
    /// the owner saw as click boxes and hover alternating per frame. With
    /// one entry per box, drawing B leaves A's answer standing.
    #[test]
    fn each_screens_box_keeps_its_own_buttons_across_the_other_screens_frame() {
        let r_tall = box_of(TALL_H * 2.0 + TALL_GAP);
        let r_short = RectC { x: 400.0, ..box_of(SHORT_H * 2.0 + SHORT_GAP) };
        let drawn_tall = button_rects(TALL_H, TALL_GAP, W_FRAC, ALIGN_BOTTOM, r_tall);
        let drawn_short = button_rects(SHORT_H, SHORT_GAP, W_FRAC, ALIGN_BOTTOM, r_short);

        let mut this = fresh();
        // The frame order of one desktop pass: screen A, then screen B.
        record_drawn(&mut this, r_tall, drawn_tall);
        record_drawn(&mut this, r_short, drawn_short);

        let for_a = stored_for(&this.drawn, r_tall)
            .expect("screen A's box was forgotten the moment screen B drew");
        assert_eq!(for_a[1].y, drawn_tall[1].y, "screen A answered with screen B's buttons");
        let for_b = stored_for(&this.drawn, r_short).expect("screen B's box was not stored");
        assert_eq!(for_b[1].y, drawn_short[1].y, "screen B answered with screen A's buttons");

        // A later frame of the same boxes REPLACES in place — two screens
        // never grow past two entries, however many frames they draw.
        record_drawn(&mut this, r_tall, drawn_tall);
        record_drawn(&mut this, r_short, drawn_short);
        assert_eq!(
            this.drawn.iter().flatten().count(),
            2,
            "redrawing the same two boxes grew the store instead of replacing"
        );
    }

    /// A store past capacity evicts the OLDEST entry and only that one.
    ///
    /// Losing the oldest box costs it nothing but the fallback path until
    /// its next frame writes it back; losing a newer one would reintroduce
    /// the alternating loop for a screen that is still drawing every frame.
    #[test]
    fn a_full_store_evicts_the_oldest_entry_first() {
        let mut this = fresh();
        let boxes: Vec<RectC> = (0..DRAWN_SLOTS as u32 + 1)
            .map(|k| RectC { x: 500.0 * k as f32, ..box_of(TALL_H * 2.0 + TALL_GAP) })
            .collect();
        let rects = |r: RectC| button_rects(TALL_H, TALL_GAP, W_FRAC, ALIGN_BOTTOM, r);
        for &b in &boxes {
            record_drawn(&mut this, b, rects(b));
        }
        assert!(
            stored_for(&this.drawn, boxes[0]).is_none(),
            "the store grew past its slots, or evicted something other than the oldest"
        );
        for &b in &boxes[1..] {
            assert!(
                stored_for(&this.drawn, b).is_some(),
                "a box still in use was evicted while the oldest survived"
            );
        }
    }
}

#[cfg(test)]
mod role_tests {
    /// The caption's role is the one `button.role` names, and the
    /// binding is followed to a family that exists — the chain
    /// `resolve_ids` walks. Spelled into the code as `type.button.*`,
    /// the binding was a second copy nobody could edit; a master that
    /// re-roles its controls now moves these two buttons with the rest.
    #[test]
    fn the_caption_role_names_a_family_the_master_declares() {
        nacelle::theme::load();
        let id = nacelle::theme::id("button.role").expect("button.role");
        let role = nacelle::theme::enum_word_of(id).expect("the binding names no word");
        assert!(!role.is_empty());
        for suffix in ["size", "min_px", "leading", "tracking", "case"] {
            let name = format!("type.{role}.{suffix}");
            assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
        }
    }
}
