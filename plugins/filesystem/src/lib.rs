//! FILESYSTEM panel — icon grid like eDEX-UI, tracks the shell's working
//! directory (from /proc/<pid>/cwd); clicking a directory cds the active
//! terminal tab, clicking a file opens it with the associated application.

use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_CAPTURE,
    ACTION_NONE, ACTION_OPEN_DIR, ACTION_OPEN_FILE, DRAG_BEGIN, DRAG_END, DRAG_MOVE, MASK_QUAD_ADD,
};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::view::scroll::{
    scrollbar, Easing, ScrollPhysics, ScrollView, ScrollbarEdge, ScrollbarLook, ScrollbarMode, Snap,
};
use nacelle::view::virt;
use nacelle::Rect;
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::Instant;

/// The interface font, as the host numbers them.
const FONT_UI: u32 = 0;

/// The intersection of two rectangles, empty when they do not meet.
/// A tile the viewport's edge cuts is clickable where it is VISIBLE and
/// nowhere else: the cut-off half lies under the panel's title band,
/// and a click there belongs to the band.
fn meet(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect::new(x, y, (a.right().min(b.right()) - x).max(0.0), (a.bottom().min(b.bottom()) - y).max(0.0))
}

// ------------------------------------------------------------------ theme
//
// ABI 5 hands the theme over as tokens. Names are stable; ids are per
// master load, so they are resolved once and thrown away whenever
// theme_epoch moves. Nothing in this file knows what a colour or a length
// IS any more — a missing token degrades through the raw defaults the ABI
// itself answers (mid-grey ink, 0.0 lengths), never through a value that
// used to be the design.

/// The seven interaction states, in the matrix's declaration order. A
/// tile is a real container now (u2 §2.10): every one rests on the
/// idle rung of the `filetile` ladder, and the pointed-at one on hover.
/// The scroll thumb wears three of them — the matrix gives
/// `scrollbar.thumb` idle, hover and dragging.
const STATE_IDLE: u32 = 0;
const STATE_HOVER: u32 = 1;
const STATE_DRAGGING: u32 = 5;
/// `filetile.row_justify` declares `pack | fill`; the baked enum is the
/// word's index in that list.
const ROW_JUSTIFY_FILL: u32 = 1;

/// Under half a device pixel of a row hanging above the viewport is not
/// a partial row, it is arithmetic: `offset / pitch` on a settled view
/// lands a hair either side of a whole number. Treating that hair as a
/// cut row would put the panel under a clip it does not need and shift
/// every tile by a fraction nobody asked for. Precision, not look — no
/// theme has an opinion on it.
const PARTIAL_ROW: f32 = 0.5;

/// The engine's raw ink — what `theme_color` answers for a missing token.
/// Kept here only for the path where the host predates ABI 5 and cannot
/// be asked at all.
const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };

const NO_COLOR: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

fn token(api: &HostApi, name: &str) -> u32 {
    (api.theme_token)(name.as_ptr(), name.len() as u32)
}

/// The WORD an enum token currently resolves to — ABI 6's appended
/// `theme_enum_word` entry. Init-time like `theme_token`: asked when the
/// ids are resolved, cached for the epoch, never in the draw loop. An
/// empty answer — a host whose table ends before the entry, a missing
/// token, a token with no word — degrades exactly like MISSING: the
/// caller draws by its pre-word guess.
fn enum_word(api: &HostApi, ctx: *mut c_void, id: u32) -> String {
    if !api.has_theme_enum_word() || id == u32::MAX {
        return String::new();
    }
    let mut buf = [0u8; 64];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
}

/// Token ids this widget draws from, resolved by NAME once per epoch.
///
/// No header tokens: `FILESYSTEM` and the cwd moved to the HOST's title
/// band through `chrome` (u2 §2.10 item 1, §6.1) — same strings, same
/// data, drawn once, trimmed once. This widget's own drawing starts at
/// the content box's first row of tiles.
struct ThemeIds {
    epoch: u32,
    // ink
    error: u32,          // severity.critical.text — an I/O error is critical, not chrome
    error_on: u32,       // severity.critical.on — legible ink over that bed
    error_fill: u32,     // severity.critical.fill — the hollow pill's bed
    error_edge: u32,     // severity.critical.edge — the hollow pill's ring
    badge_border: u32,   // badge.border — the hollow pill's ring width
    /// Whether the error pill draws solid — `severity.critical.badge_style`'s
    /// WORD (ABI 6), no longer this arrangement's guess. `hatched` and
    /// `hollow_dashed` degrade to hollow, as the host's `ui::badge` degrades
    /// them; no word at all (an old host, a missing token) keeps the
    /// pre-word guess: solid, the master's own critical style.
    error_solid: bool,
    glyph_dir: u32,      // component.file.glyph_dir
    glyph_file: u32,     // component.file.glyph_file
    glyph_link: u32,     // component.file.glyph_link
    glyph_detail: u32,   // component.file.glyph_detail — the fold lines
    caption_dir: u32,    // text.primary
    caption_file: u32,   // text.secondary
    // the I/O error, an alert.banner string in a solid critical pill
    banner_size: u32,     // type.alert.banner.size
    banner_min: u32,      // type.alert.banner.min_px
    banner_tracking: u32, // type.alert.banner.tracking
    banner_leading: u32,  // type.alert.banner.leading
    banner_case: u32,     // type.alert.banner.case
    badge_h: u32,         // badge.h
    badge_pad: u32,       // badge.pad_x
    badge_corner: u32,    // badge.corner — `pill` bakes negative and squares off
    empty_y: u32,        // emptystate.y_frac
    // form
    gap: u32,            // filetile.gap
    rows: u32,           // filetile.rows
    cols: u32,           // filetile.cols — a count, or the `auto` sentinel
    cell_min: u32,       // filetile.cell_min_px
    cell_pref: u32,      // filetile.cell — preferred tile edge; 0 = size from rows
    corner: u32,           // filetile.corner — the tile container's chamfer cut
    caption_size: u32,     // type.tooltip.size, via filetile.caption_role
    caption_tracking: u32, // type.tooltip.tracking
    caption_gap: u32,      // filetile.caption_gap
    icon_inset_x: u32,     // filetile.icon.inset_x
    icon_inset_y: u32,     // filetile.icon.inset_y
    icon_w: u32,           // filetile.icon.w
    icon_h: u32,           // filetile.icon.h
    icon_stroke: u32,      // filetile.icon.stroke
    detail_stroke: u32,    // icon.stroke_thin
    wheel: u32,            // filetile.wheel_px
    row_justify: u32,      // filetile.row_justify
    // the scrollbar — the spec's new component, drawn at last: a panel
    // that scrolls without saying where it is is a defect (u2 §2.10)
    sb_w: u32,         // scrollbar.w
    sb_w_hover: u32,   // scrollbar.w_hover — the width under the pointer
    sb_margin: u32,    // scrollbar.margin
    sb_thumb_min: u32, // scrollbar.thumb_min
    sb_auto_hide: u32, // scrollbar.auto_hide
    sb_fade: u32,      // scrollbar.fade_ms
    /// `scrollbar.mode` and `scrollbar.edge` as their WORDS (ABI 6).
    /// An index is a position in a list this file cannot see; the word
    /// is what both sides of the boundary can compare, and it is what
    /// lets `none` mean none instead of "not overlay". No word at all —
    /// an old host — leaves the pre-word guess: index 0 is the first
    /// word the token declares, `overlay` and `right`.
    sb_mode_word: String,
    sb_edge_word: String,
    // the physics — `view::ScrollView` reads none of these itself; a
    // plugin fills the same struct the host fills, from its own ids
    scroll_fling: u32,     // scroll.fling_scale
    scroll_halflife: u32,  // scroll.glide_halflife_ms
    settle_enabled: u32,   // motion.scroll_settle.enabled
    settle_ms: u32,        // motion.scroll_settle.duration_ms
    settle_duty: u32,      // motion.scroll_settle.duty — step easing only
    settle_floor: u32,     // motion.scroll_settle.floor — step easing only
    motion_scale: u32,     // motion.scale — 0 is reduced motion, and freezes
    /// `motion.scroll_settle.easing` as its word, resolved per epoch.
    settle_word: String,
    /// The tile's row in the class × state matrix.
    tile_class: u32,
    /// The scroll thumb's row in the same matrix.
    thumb_class: u32,
    // The glow class the tile's ring wears — `shape.icon_tile.glow`, the
    // master's `@glow.icon_idle` (image 1's bordered launcher squares).
    // The reference crosses ABI 6 as its WORD ("glow.icon_idle"), and the
    // class's own tokens are resolved from it, so which class glows is the
    // theme's sentence, never this file's. No word — the `none` other
    // presets carry, an old host — resolves nothing and the ring stays bare.
    tile_glow_enabled: u32, // glow.<class>.enabled
    tile_glow_radius: u32,  // glow.<class>.radius
    tile_glow_alpha: u32,   // glow.<class>.alpha
    glow_scale: u32,        // glow.alpha_scale — the one global knob
}

impl ThemeIds {
    fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> ThemeIds {
        let style_word = enum_word(api, ctx, token(api, "severity.critical.badge_style"));
        // Without `mask_quad` the glow cannot be drawn at all, so the
        // class is not even asked for — the same degrade as `none`.
        let glow_class = if api.has_mask_quad() {
            enum_word(api, ctx, token(api, "shape.icon_tile.glow"))
        } else {
            String::new()
        };
        let g = |suffix: &str| {
            if glow_class.is_empty() {
                u32::MAX
            } else {
                token(api, &format!("{glow_class}.{suffix}"))
            }
        };
        // Three enum tokens the WORD decides, asked once per epoch like
        // every other word here: an index is a position in a list this
        // side cannot see, and `none` must not read as "not overlay".
        let mode_word = enum_word(api, ctx, token(api, "scrollbar.mode"));
        let edge_word = enum_word(api, ctx, token(api, "scrollbar.edge"));
        let settle_word = enum_word(api, ctx, token(api, "motion.scroll_settle.easing"));
        ThemeIds {
            epoch,
            error: token(api, "severity.critical.text"),
            error_on: token(api, "severity.critical.on"),
            error_fill: token(api, "severity.critical.fill"),
            error_edge: token(api, "severity.critical.edge"),
            badge_border: token(api, "badge.border"),
            error_solid: style_word.is_empty() || style_word == "solid",
            tile_glow_enabled: g("enabled"),
            tile_glow_radius: g("radius"),
            tile_glow_alpha: g("alpha"),
            glow_scale: token(api, "glow.alpha_scale"),
            glyph_dir: token(api, "component.file.glyph_dir"),
            glyph_file: token(api, "component.file.glyph_file"),
            glyph_link: token(api, "component.file.glyph_link"),
            glyph_detail: token(api, "component.file.glyph_detail"),
            caption_dir: token(api, "text.primary"),
            caption_file: token(api, "text.secondary"),
            banner_size: token(api, "type.alert.banner.size"),
            banner_min: token(api, "type.alert.banner.min_px"),
            banner_tracking: token(api, "type.alert.banner.tracking"),
            banner_leading: token(api, "type.alert.banner.leading"),
            banner_case: token(api, "type.alert.banner.case"),
            badge_h: token(api, "badge.h"),
            badge_pad: token(api, "badge.pad_x"),
            badge_corner: token(api, "badge.corner"),
            empty_y: token(api, "emptystate.y_frac"),
            gap: token(api, "filetile.gap"),
            rows: token(api, "filetile.rows"),
            cols: token(api, "filetile.cols"),
            cell_min: token(api, "filetile.cell_min_px"),
            cell_pref: token(api, "filetile.cell"),
            corner: token(api, "filetile.corner"),
            caption_size: token(api, "type.tooltip.size"),
            caption_tracking: token(api, "type.tooltip.tracking"),
            caption_gap: token(api, "filetile.caption_gap"),
            icon_inset_x: token(api, "filetile.icon.inset_x"),
            icon_inset_y: token(api, "filetile.icon.inset_y"),
            icon_w: token(api, "filetile.icon.w"),
            icon_h: token(api, "filetile.icon.h"),
            icon_stroke: token(api, "filetile.icon.stroke"),
            detail_stroke: token(api, "icon.stroke_thin"),
            wheel: token(api, "filetile.wheel_px"),
            row_justify: token(api, "filetile.row_justify"),
            sb_w: token(api, "scrollbar.w"),
            sb_w_hover: token(api, "scrollbar.w_hover"),
            sb_margin: token(api, "scrollbar.margin"),
            sb_thumb_min: token(api, "scrollbar.thumb_min"),
            sb_auto_hide: token(api, "scrollbar.auto_hide"),
            sb_fade: token(api, "scrollbar.fade_ms"),
            sb_mode_word: mode_word,
            sb_edge_word: edge_word,
            scroll_fling: token(api, "scroll.fling_scale"),
            scroll_halflife: token(api, "scroll.glide_halflife_ms"),
            settle_enabled: token(api, "motion.scroll_settle.enabled"),
            settle_ms: token(api, "motion.scroll_settle.duration_ms"),
            settle_duty: token(api, "motion.scroll_settle.duty"),
            settle_floor: token(api, "motion.scroll_settle.floor"),
            motion_scale: token(api, "motion.scale"),
            settle_word,
            tile_class: (api.theme_class)("filetile".as_ptr(), "filetile".len() as u32),
            thumb_class: (api.theme_class)(
                "scrollbar.thumb".as_ptr(),
                "scrollbar.thumb".len() as u32,
            ),
        }
    }
}

/// The values one frame draws with, read fresh from the resolved ids.
/// Colours and lengths only — nothing here is arithmetic on anything.
struct Look {
    error: ColorC,
    error_on: ColorC,
    error_fill: ColorC,
    error_edge: ColorC,
    badge_border: f32,
    error_solid: bool,
    /// The tile ring's glow, mirrored from `object::window::panel_edge_glow`:
    /// the class flag, the reach, and `alpha * glow.alpha_scale` already
    /// folded. Off — the default, every class disabled — is three zeros.
    glow_on: bool,
    glow_radius: f32,
    glow_alpha: f32,
    glyph_dir: ColorC,
    glyph_file: ColorC,
    glyph_link: ColorC,
    glyph_detail: ColorC,
    caption_dir: ColorC,
    caption_file: ColorC,
    idle: StateStyleC,
    hover: StateStyleC,
    /// The thumb's three rungs — the matrix declares idle, hover and
    /// dragging for `scrollbar.thumb`, and until it could be grabbed
    /// this widget only ever drew the first.
    thumb: StateStyleC,
    thumb_hover: StateStyleC,
    thumb_drag: StateStyleC,
    banner_px: f32,
    banner_tracking: f32,
    banner_leading: f32,
    banner_case: u32,
    badge_h: f32,
    badge_pad: f32,
    badge_corner: f32,
    empty_y: f32,
    gap: f32,
    rows: f32,
    cols: f32,
    cell_min: f32,
    cell_pref: f32,
    corner: f32,
    caption_px: f32,
    caption_tracking: f32,
    caption_gap: f32,
    icon_inset_x: f32,
    icon_inset_y: f32,
    icon_w: f32,
    icon_h: f32,
    icon_stroke: f32,
    detail_stroke: f32,
    wheel_px: f32,
    row_justify: u32,
    /// Everything the scroll offset's physics reads from the theme, in
    /// the toolkit's own struct: the host fills it from `TokenId`s, a
    /// plugin from its ABI ids, and `ScrollView` cannot tell which side
    /// it is running on. `wheel_px` is this widget's own
    /// `filetile.wheel_px` — a tile grid names its notch, and the
    /// generic `scroll.wheel_px` is three rows of TEXT.
    physics: ScrollPhysics,
    /// Everything the bar's geometry reads, likewise.
    bar: ScrollbarLook,
}

impl Look {
    /// The pre-token world: a host that answers no theme calls at all.
    /// Grey ink, zero lengths — the engine's kind defaults, mirrored, so
    /// an old host shows the same undesigned raw as an empty theme.
    fn raw() -> Look {
        let raw_state = StateStyleC {
            fill: NO_COLOR,
            edge: RAW_INK,
            text: RAW_INK,
            glyph: RAW_INK,
            edge_width: 1.0,
            glow_radius: 0.0,
            glow_alpha: 0.0,
            elevation: 0.0,
        };
        Look {
            error: RAW_INK,
            error_on: RAW_INK,
            error_fill: NO_COLOR,
            error_edge: RAW_INK,
            badge_border: 0.0,
            // The raw pill keeps the pre-word arrangement: solid.
            error_solid: true,
            glow_on: false,
            glow_radius: 0.0,
            glow_alpha: 0.0,
            glyph_dir: RAW_INK,
            glyph_file: RAW_INK,
            glyph_link: RAW_INK,
            glyph_detail: RAW_INK,
            caption_dir: RAW_INK,
            caption_file: RAW_INK,
            idle: raw_state,
            hover: raw_state,
            thumb: raw_state,
            thumb_hover: raw_state,
            thumb_drag: raw_state,
            banner_px: 0.0,
            banner_tracking: 0.0,
            banner_leading: 1.0,
            banner_case: 0,
            badge_h: 0.0,
            badge_pad: 0.0,
            badge_corner: 0.0,
            empty_y: 0.0,
            gap: 0.0,
            rows: 0.0,
            cols: 0.0,
            cell_min: 0.0,
            cell_pref: 0.0,
            corner: 0.0,
            caption_px: 0.0,
            caption_tracking: 0.0,
            caption_gap: 0.0,
            icon_inset_x: 0.0,
            icon_inset_y: 0.0,
            icon_w: 0.0,
            icon_h: 0.0,
            icon_stroke: 0.0,
            detail_stroke: 0.0,
            wheel_px: 0.0,
            row_justify: 0,
            // A host with no theme has no motion either: zero lengths
            // mean a wheel that moves nothing, and `motion_scale = 0`
            // is the reduced-motion answer — nothing glides, nothing
            // settles, which is the honest raw for a view that cannot
            // ask what a millisecond is worth.
            physics: ScrollPhysics {
                wheel_px: 0.0,
                fling_scale: 0.0,
                glide_halflife_ms: 0.0,
                settle_ms: 0.0,
                settle_easing: Easing::Linear,
                motion_scale: 0.0,
            },
            // A zero-width bar draws nothing, which is what a host that
            // cannot be asked for `scrollbar.w` has always got.
            bar: ScrollbarLook {
                mode: ScrollbarMode::Overlay,
                w: 0.0,
                w_hover: 0.0,
                margin: 0.0,
                thumb_min: 0.0,
                edge: ScrollbarEdge::Right,
                auto_hide: false,
                fade_ms: 0.0,
            },
        }
    }

    fn read(api: &HostApi, ctx: *mut c_void, t: &ThemeIds) -> Look {
        let col = |id| (api.theme_color)(ctx, id);
        let px = |id| (api.theme_px)(ctx, id);
        // A colour used as a BED — the hollow pill's interior. Missing,
        // it answers the engine's raw near-black rather than the mid grey.
        let bed = |id| (api.theme_bed)(ctx, id);
        let flag = |id| (api.theme_flag)(ctx, id) != 0;
        // A rung of a class's ladder, whole. A missing class answers the
        // matrix's own raw rung, so no fallback lives here.
        let rung = |class: u32, state: u32| {
            let mut out = StateStyleC {
                fill: NO_COLOR,
                edge: NO_COLOR,
                text: NO_COLOR,
                glyph: NO_COLOR,
                edge_width: 0.0,
                glow_radius: 0.0,
                glow_alpha: 0.0,
                elevation: 0.0,
            };
            (api.theme_class_state)(
                ctx,
                class,
                state,
                &mut out,
                std::mem::size_of::<StateStyleC>() as u32,
            );
            out
        };
        Look {
            error: col(t.error),
            error_on: col(t.error_on),
            error_fill: bed(t.error_fill),
            error_edge: col(t.error_edge),
            badge_border: px(t.badge_border),
            error_solid: t.error_solid,
            glow_on: flag(t.tile_glow_enabled),
            glow_radius: px(t.tile_glow_radius),
            glow_alpha: (px(t.tile_glow_alpha) * px(t.glow_scale)).clamp(0.0, 1.0),
            glyph_dir: col(t.glyph_dir),
            glyph_file: col(t.glyph_file),
            glyph_link: col(t.glyph_link),
            glyph_detail: col(t.glyph_detail),
            caption_dir: col(t.caption_dir),
            caption_file: col(t.caption_file),
            idle: rung(t.tile_class, STATE_IDLE),
            hover: rung(t.tile_class, STATE_HOVER),
            thumb: rung(t.thumb_class, STATE_IDLE),
            thumb_hover: rung(t.thumb_class, STATE_HOVER),
            thumb_drag: rung(t.thumb_class, STATE_DRAGGING),
            banner_px: px(t.banner_size).max(px(t.banner_min)),
            banner_tracking: px(t.banner_tracking),
            banner_leading: px(t.banner_leading).max(1.0),
            banner_case: (api.theme_enum)(ctx, t.banner_case),
            badge_h: px(t.badge_h),
            badge_pad: px(t.badge_pad),
            badge_corner: px(t.badge_corner),
            empty_y: px(t.empty_y),
            gap: px(t.gap),
            rows: px(t.rows),
            cols: px(t.cols),
            cell_min: px(t.cell_min),
            cell_pref: px(t.cell_pref),
            corner: px(t.corner),
            caption_px: px(t.caption_size),
            caption_tracking: px(t.caption_tracking),
            caption_gap: px(t.caption_gap),
            icon_inset_x: px(t.icon_inset_x),
            icon_inset_y: px(t.icon_inset_y),
            icon_w: px(t.icon_w),
            icon_h: px(t.icon_h),
            icon_stroke: px(t.icon_stroke),
            detail_stroke: px(t.detail_stroke),
            wheel_px: px(t.wheel),
            row_justify: (api.theme_enum)(ctx, t.row_justify),
            physics: ScrollPhysics {
                // This grid's own notch, not the generic
                // `scroll.wheel_px`: that one is three rows of a TEXT
                // table, and a row here is a tile eight times as tall.
                wheel_px: px(t.wheel),
                fling_scale: px(t.scroll_fling),
                glide_halflife_ms: px(t.scroll_halflife),
                // The effect's switch decides whether there is a
                // settle at all; the duration alone would keep
                // animating a disabled effect.
                settle_ms: if flag(t.settle_enabled) { px(t.settle_ms) } else { 0.0 },
                settle_easing: match t.settle_word.as_str() {
                    "ease_out" => Easing::EaseOut,
                    "ease_in" => Easing::EaseIn,
                    "ease_in_out" => Easing::EaseInOut,
                    "sine" => Easing::Sine,
                    "step" => Easing::Step {
                        duty: px(t.settle_duty),
                        floor: px(t.settle_floor),
                    },
                    // No word — a host from before `theme_enum_word`,
                    // or a curve this build has never heard of — is the
                    // enum's own fallback, a straight line.
                    _ => Easing::Linear,
                },
                motion_scale: px(t.motion_scale),
            },
            bar: ScrollbarLook {
                mode: match t.sb_mode_word.as_str() {
                    "none" => ScrollbarMode::None,
                    // `inset` is honoured as far as this widget can:
                    // the bar is drawn, but the tile grid is laid out
                    // before there is a bar to make room for, so it
                    // costs no width yet. Said here rather than
                    // silently drawing nothing, which is what the
                    // index-only path did.
                    "inset" => ScrollbarMode::Inset,
                    _ => ScrollbarMode::Overlay,
                },
                w: px(t.sb_w),
                w_hover: px(t.sb_w_hover),
                margin: px(t.sb_margin),
                thumb_min: px(t.sb_thumb_min),
                edge: if t.sb_edge_word == "left" {
                    ScrollbarEdge::Left
                } else {
                    ScrollbarEdge::Right
                },
                auto_hide: flag(t.sb_auto_hide),
                fade_ms: px(t.sb_fade),
            },
        }
    }
}

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

fn measure(api: &HostApi, ctx: *mut c_void, px: f32, text: &str, spacing: f32) -> f32 {
    (api.measure)(ctx, FONT_UI, px, text.as_ptr(), text.len() as u32, spacing)
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    api: &HostApi,
    ctx: *mut c_void,
    px: f32,
    x: f32,
    y: f32,
    text: &str,
    c: ColorC,
    spacing: f32,
    align: u32,
) {
    (api.text)(
        ctx,
        FONT_UI,
        px,
        x,
        y,
        text.as_ptr(),
        text.len() as u32,
        c,
        spacing,
        align,
    );
}

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
}

/// Click event in the file panel.
pub enum FsEvent {
    /// Entering a directory — the active terminal tab should `cd`.
    OpenDir(PathBuf),
    /// Opening a file with the system-associated application (xdg-open).
    OpenFile(PathBuf),
}

/// The bar as the last frame drew it, and the two lengths its arithmetic
/// needs: a press, a motion and a release all arrive with no drawing
/// context, and must be answered in the geometry the hand is actually
/// looking at. The wheel's own distance has been cached this way since
/// this panel grew a wheel; a thumb that can be taken hold of needs the
/// rest of the picture too.
///
/// A page and the whole content are both measured in ROWS (`nvis` of
/// them and all of them): those are the two numbers the hand-rolled
/// thumb divided, so the thumb keeps exactly the length and the travel
/// it has always had.
#[derive(Clone, Copy)]
struct Bar {
    viewport: f32,
    content: f32,
    /// The full length the thumb travels in, in window coordinates —
    /// the same space a press arrives in.
    track: Rect,
    /// The thumb AS DRAWN: `scrollbar.thumb_min` may have stretched it
    /// and the pointer may have widened it, and a grab must be tested
    /// against what the eye saw.
    thumb: Rect,
}

pub struct Filesystem {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    /// Where the panel is scrolled to, and everything being done to it:
    /// the wheel, a thumb under the hand, a theme's glide and settle.
    /// The offset is in the SAME units the tiles are laid out in — one
    /// row is `filetile.cell + filetile.gap`, or the pitch
    /// `row_justify = fill` stretched it to.
    view: ScrollView,
    /// The path the last click produced, kept alive until the next one.
    last_path: Vec<u8>,
    /// Tile rectangles from the last frame.
    hits: Vec<(Rect, usize)>,
    last_refresh: Instant,
    error: Option<String>,
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<ThemeIds>,
    /// The physics the last frame read, cached because a wheel event
    /// arrives with no drawing context to ask the theme through.
    physics: ScrollPhysics,
    /// The bar the last frame drew, or none when there was nothing to
    /// scroll.
    bar: Option<Bar>,
    /// The host's clock at the last draw. Every event this widget
    /// answers happens between two frames, so the frame's time is the
    /// closest thing to "now" it can honestly use.
    frame_t: f64,
    /// The cwd as last handed to the host's title band, alive until the
    /// next `chrome` call — the same lifetime promise `last_path` makes
    /// for the click path.
    chrome_right: Vec<u8>,
}

impl Filesystem {
    pub fn new(start: PathBuf) -> Self {
        let mut fs = Filesystem {
            cwd: start,
            entries: Vec::new(),
            view: ScrollView::new(),
            last_path: Vec::new(),
            hits: Vec::new(),
            last_refresh: Instant::now() - std::time::Duration::from_secs(60),
            error: None,
            theme: None,
            physics: Look::raw().physics,
            bar: None,
            frame_t: 0.0,
            chrome_right: Vec::new(),
        };
        fs.refresh();
        fs
    }

    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        self.entries.clear();
        self.error = None;
        if self.cwd.parent().is_some() {
            self.entries.push(Entry {
                name: "..".into(),
                is_dir: true,
                is_link: false,
            });
        }
        match std::fs::read_dir(&self.cwd) {
            Ok(rd) => {
                let mut list: Vec<Entry> = rd
                    .flatten()
                    .filter_map(|e| {
                        let ft = e.file_type().ok()?;
                        let is_link = ft.is_symlink();
                        // Follow links (symbolic and otherwise): the target's
                        // type decides whether it is treated as a directory.
                        let target = std::fs::metadata(e.path()).ok();
                        let is_dir =
                            target.as_ref().map(|m| m.is_dir()).unwrap_or(ft.is_dir());
                        Some(Entry {
                            name: e.file_name().to_string_lossy().into_owned(),
                            is_dir,
                            is_link,
                        })
                    })
                    .collect();
                list.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.entries.extend(list);
            }
            Err(e) => self.error = Some(format!("I/O ERROR: {e}")),
        }
    }

    /// Following the shell's directory.
    pub fn follow(&mut self, shell_cwd: Option<PathBuf>) {
        if let Some(cwd) = shell_cwd {
            if cwd != self.cwd {
                self.cwd = cwd;
                // A new directory is a new model, not an interaction:
                // back to the top with nothing in flight.
                self.view.reset();
                self.refresh();
            }
        }
        if self.last_refresh.elapsed().as_secs() >= 2 {
            self.refresh();
        }
    }

    /// The wheel turned. Positive `notches` is the direction the host
    /// spells as "toward the top of the list", which is why the sign
    /// flips on the way into the view: a scroll view counts pixels from
    /// the top of the content.
    ///
    /// The distance a notch travels is `filetile.wheel_px`, exactly as
    /// it always was; whether it lands or glides is
    /// `scroll.fling_scale`, and the master ships it at 0 — a notch
    /// moves the offset and stays there.
    pub fn wheel(&mut self, notches: f32) {
        let p = self.physics;
        self.view.wheel(-notches, &p, self.frame_t);
    }

    /// A pointer press. `true` when the widget took the gesture — the
    /// host then captures the pointer, the board does not turn under
    /// the hand and no click is delivered when it is let go.
    ///
    /// Only the bar takes a press: everything else in this panel is a
    /// tile, and a tile is opened on the RELEASE, by `click`, exactly
    /// as it always has been.
    pub fn press(&mut self, x: f32, y: f32) -> bool {
        let Some(bar) = self.bar else { return false };
        if !bar.track.contains(x, y) {
            return false;
        }
        if self.view.press_thumb(y, bar.thumb) {
            return true;
        }
        // Beside the thumb: one page toward the click. The press is
        // still taken — the overlay bar lies ON TOP of the tiles, and
        // letting it through would open a file the hand never aimed at.
        self.view
            .page(y >= bar.thumb.y + bar.thumb.h, bar.viewport, self.frame_t);
        true
    }

    /// The pointer moved while it held the thumb. Only the y matters:
    /// the thumb goes where the hand is, and a hand that wandered off
    /// the bar sideways is still holding it.
    pub fn drag_to(&mut self, y: f32) {
        if let Some(bar) = self.bar {
            self.view.drag(y, bar.viewport, bar.content, bar.track);
        }
    }

    /// The pointer let go. The next frame lands the view on its nearest
    /// legal stop through `motion.scroll_settle` — a whole row on a
    /// host that cannot clip, where whole rows are the only legal stop,
    /// and where the hand left it on one that can.
    pub fn release(&mut self) {
        self.view.release();
    }

    /// Click; returns an event to be handled by the main loop.
    pub fn click(&mut self, x: f32, y: f32) -> Option<FsEvent> {
        let idx = self
            .hits
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, i)| i)?;
        let entry = self.entries.get(idx)?.clone();
        if entry.is_dir {
            let target = if entry.name == ".." {
                self.cwd.parent()?.to_path_buf()
            } else {
                self.cwd.join(&entry.name)
            };
            self.view.reset();
            Some(FsEvent::OpenDir(target))
        } else {
            Some(FsEvent::OpenFile(self.cwd.join(&entry.name)))
        }
    }

    /// This frame's theme values. Ids are cached across frames; the values
    /// are read fresh, because they are what a mood or a resize changes.
    fn look(&mut self, api: &HostApi, ctx: *mut c_void) -> Look {
        // ABI 5 is where the token entries live. attach() refuses an older
        // host outright, so this branch is belt and braces for the day the
        // check moves — an old table simply ends before these entries do.
        if api.abi_version < 5 {
            return Look::raw();
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|t| t.epoch) != Some(epoch) {
            self.theme = Some(ThemeIds::resolve(api, ctx, epoch));
        }
        match &self.theme {
            Some(t) => Look::read(api, ctx, t),
            None => Look::raw(),
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        // The bar goes the same way as the hits: a frame that draws no
        // grid — an I/O error page — has no bar to press either, and a
        // press must never be answered in last frame's geometry.
        self.bar = None;
        let look = self.look(api, ctx);
        self.physics = look.physics;
        self.frame_t = (api.elapsed)(ctx);

        // No header here: `FILESYSTEM` and the cwd are the HOST's title
        // band, from `chrome` (u2 §2.10 item 1) — the band does the left
        // trim in the one place that draws it. What used to be a local
        // `module_title` and a second `fit_tail` is gone, and the whole
        // content box belongs to the tiles.

        if let Some(err) = &self.error {
            // An I/O error is critical, not chrome: an alert.banner
            // string in a severity.critical pill (u2 §2.10). Its
            // arrangement is the severity's own badge_style WORD (ABI 6)
            // — the solid the master declares for critical, or the
            // hollow a theme may say instead — no longer this file's
            // guess; the indices alone could not tell the styles apart.
            // Solid mirrors `ui::badge`: the text colour is the bed and
            // `on` is the ink. Hollow is the ring-and-bed form the
            // SCROLL pill wears.
            let text = recase(look.banner_case, err.clone());
            let bpx = look.banner_px;
            let track = bpx * look.banner_tracking;
            let tw = measure(api, ctx, bpx, &text, track);
            let w = (tw + 2.0 * look.badge_pad).min(r.w).max(1.0);
            let h = look.badge_h.max(1.0);
            let pill = RectC {
                x: r.cx() - w / 2.0,
                y: r.y + r.h * look.empty_y - h / 2.0,
                w,
                h,
            };
            let (fill, ink) = if look.error_solid {
                (look.error, look.error_on)
            } else {
                (look.error_fill, look.error)
            };
            let cut = look.badge_corner;
            if cut > 0.0 {
                let cut = cut.min(h / 2.0);
                chamfer_fill(api, ctx, pill, cut, fill);
                if !look.error_solid && look.badge_border > 0.0 {
                    chamfer_frame(api, ctx, pill, cut, look.badge_border, look.error_edge);
                }
            } else {
                // `pill` is a negative sentinel until R5 lands: square.
                (api.rect)(ctx, pill, fill);
                if !look.error_solid && look.badge_border > 0.0 {
                    (api.rect_outline)(ctx, pill, look.badge_border, look.error_edge);
                }
            }
            draw_text(
                api,
                ctx,
                bpx,
                pill.x + pill.w / 2.0,
                pill.y + (pill.h - bpx * look.banner_leading) / 2.0,
                &text,
                ink,
                track,
                1,
            );
            return;
        }

        // The tile grid; further rows reachable by scrolling. The area's
        // bottom matches the panel (and keyboard) bottom edge.
        let area = Rect::new(r.x, r.y, r.w, r.h);
        let gap = look.gap;
        let rows_page = look.rows.max(1.0);
        // The cell a full page of rows asks for: filetile.rows of them
        // fill the area's height.
        // filetile.cell names the tile edge directly; rows-per-page is the
        // fallback for a theme that sizes by page count instead. Image 8's
        // rows = 3 was written for a WIDE panel — on this tall column it
        // meant three giant tiles, which is what the cell token undoes.
        let row_cell = if look.cell_pref > 0.0 {
            look.cell_pref.max(look.cell_min)
        } else {
            ((area.h - gap * (rows_page - 1.0)) / rows_page).max(look.cell_min)
        };
        // filetile.cols: a count, or the `auto` sentinel (< 1), which fits
        // as many row-sized cells as the width allows.
        let cols = if look.cols >= 1.0 {
            look.cols.round() as usize
        } else if row_cell + gap > 0.0 {
            (((area.w + gap) / (row_cell + gap)).floor() as usize).max(1)
        } else {
            1
        };
        // Never taller than the content box: at least one row is always
        // drawn (nvis is floored at 1), so a tile sized only by the
        // width would overrun the panel's bottom edge on a squeezed
        // portrait column and paint over the neighbour below. The
        // cell_min floor still wins — below it a tile stops being
        // legible, and the layout's minimums keep the box above it.
        let tile = ((area.w - gap * (cols as f32 - 1.0)) / cols as f32)
            .min(row_cell)
            .min(area.h.max(look.cell_min))
            .max(look.cell_min);
        let name_px = look.caption_px;
        let name_sp = name_px * look.caption_tracking;

        // How many rows there are and how many of them fit whole.
        let row_h = tile + gap;
        let total_rows = self.entries.len().div_ceil(cols);
        let nvis = if row_h > 0.0 {
            (((area.h + gap) / row_h).floor() as usize).max(1)
        } else {
            1
        };
        let max_off = total_rows.saturating_sub(nvis);
        // filetile.row_justify = fill stretches the pitch so the last
        // visible row ends exactly at the panel's bottom edge (level with
        // the keyboard); pack sits every row on filetile.gap.
        let step = if look.row_justify == ROW_JUSTIFY_FILL && total_rows > nvis && nvis > 1
        {
            (area.h - tile) / (nvis as f32 - 1.0)
        } else {
            row_h
        };

        // The panel scrolls in ROWS, so it measures itself in rows: a
        // page is `nvis` of them, the content is all of them, and the
        // furthest the offset may go is `max_off` of them — the very
        // clamp this widget has always applied. The thumb divides those
        // two numbers, so it keeps the length and the travel it had.
        let viewport = nvis as f32 * step;
        let content = total_rows as f32 * step;
        // Whole rows are the only legal resting place when the host
        // cannot clip: a row the viewport's edge cuts would paint over
        // the panel below it. A host that CAN clip lets the offset rest
        // anywhere, which is what makes a dragged thumb follow the hand
        // instead of jumping under it.
        let can_clip = api.has_clip();
        let snap = if can_clip { Snap::None } else { Snap::Row(step) };
        if !can_clip {
            // ... and then a notch smaller than half a row could never
            // cross a boundary, and the panel would not move at all. On
            // such a host a notch is a row.
            self.physics.wheel_px = look.wheel_px.max(step);
        }
        let physics = self.physics;
        self.view.tick(self.frame_t, viewport, content, snap, &physics);
        let offset = self.view.offset();

        let (first, count, y0) =
            row_span(offset, area.h, step, total_rows, nvis, max_off, can_clip);
        // A partial row exists only under a clip, and a clip is pushed
        // only when there is one: at rest the draw list is the one it
        // has always been, down to the command.
        let clipping = y0 < 0.0;
        if clipping {
            (api.push_clip)(ctx, RectC { x: area.x, y: area.y, w: area.w, h: area.h });
        }

        // Where the pointer is, once per frame; NaN matches no tile.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);

        // Only the window is walked. Four thousand entries used to cost
        // four thousand iterations a frame to skip all but a dozen;
        // virtualisation is exactly this loop reaching the rows the eye
        // can see and no others.
        let lo = first * cols;
        let hi = ((first + count) * cols).min(self.entries.len());
        for i in lo..hi {
            let entry = &self.entries[i];
            let col = i % cols;
            let row = i / cols;
            let x = area.x + col as f32 * (tile + gap);
            let y = area.y + y0 + (row - first) as f32 * step;
            let trect = Rect::new(x, y, tile, tile);
            // The tile is a real container (u2 §2.10): every one rests
            // on the filetile ladder's idle rung — a bordered cell with
            // the glyph inset and the caption under it, image 1's
            // launcher cell — and the pointed-at one takes hover. The
            // corner honours filetile.corner as far as the renderer can:
            // a positive cut chamfers, rounding is R5's.
            let rung = if trect.contains(mx, my) { &look.hover } else { &look.idle };
            let cell = RectC { x, y, w: tile, h: tile };
            let cut = look.corner.min(tile / 2.0);
            if rung.fill.a > 0.0 {
                if cut > 0.0 {
                    chamfer_fill(api, ctx, cell, cut, rung.fill);
                } else {
                    (api.rect)(ctx, cell, rung.fill);
                }
            }
            if rung.edge_width > 0.0 && rung.edge.a > 0.0 {
                if cut > 0.0 {
                    chamfer_frame(api, ctx, cell, cut, rung.edge_width, rung.edge);
                } else {
                    (api.rect_outline)(ctx, cell, rung.edge_width, rung.edge);
                }
                // Right after the stroke, the ring's glow — the class
                // `shape.icon_tile.glow` names (ABI 6), tinted with the
                // edge's own resolved colour (the `element` rule, the
                // only arm the host honours either) at the class's alpha
                // times `glow.alpha_scale`: `panel_edge_glow`'s recipe,
                // reached over `mask_quad`. Every shipped default is
                // off; a theme opts a class in and the squares light up.
                if look.glow_on && look.glow_radius > 0.0 && look.glow_alpha > 0.0 {
                    let c = ColorC { a: look.glow_alpha, ..rung.edge };
                    chamfer_glow(api, ctx, cell, cut, look.glow_radius, c);
                }
            }

            // Icon drawn as vectors, placed by the filetile.icon.* box.
            // These are the icon registry's `folder` and `document`
            // slots' BUILT-IN FALLBACK (icon.fallback = builtin): the
            // engine does not bake icon.<name>.layers across the ABI
            // yet, so the compiled-in recipe is the drawn form, in the
            // component.file.* colours the theme names. When the layer
            // path lands, a theme's [ U+E210 @data.series[7], … ] takes
            // over and this recipe stays as what an unmapped name draws.
            let icon = Rect::new(
                x + tile * look.icon_inset_x,
                y + tile * look.icon_inset_y,
                tile * look.icon_w,
                tile * look.icon_h,
            );
            if entry.is_dir {
                draw_folder_icon(api, ctx, icon, look.glyph_dir, look.icon_stroke);
            } else {
                draw_file_icon(
                    api,
                    ctx,
                    icon,
                    look.glyph_file,
                    look.glyph_detail,
                    look.icon_stroke,
                    look.detail_stroke,
                );
            }
            if entry.is_link {
                // The symlink tick — icon.link_badge.layers' documented
                // default: the slot ships empty, which KEEPS this drawn
                // diagonal in component.file.glyph_link; a theme fills
                // the slot to replace the mark with a glyph.
                (api.line)(
                    ctx,
                    icon.x,
                    icon.bottom(),
                    icon.x + icon.w * 0.3,
                    icon.bottom() - icon.h * 0.3,
                    look.icon_stroke,
                    look.glyph_link,
                );
            }

            // Name under the icon, trimmed by measured width.
            let name = fit_name(api, ctx, name_px, &entry.name, tile, name_sp);
            draw_text(
                api,
                ctx,
                name_px,
                trect.cx(),
                y + tile * look.caption_gap,
                &name,
                if entry.is_dir { look.caption_dir } else { look.caption_file },
                name_sp,
                1,
            );

            // Clickable where it is VISIBLE: a row the viewport's edge
            // cuts keeps only the part inside the content box, so a
            // click on the title band above it stays the band's.
            let hit = meet(trect, area);
            if hit.w > 0.0 && hit.h > 0.0 {
                self.hits.push((hit, i));
            }
        }
        if clipping {
            (api.pop_clip)(ctx);
        }

        // The scrollbar, drawn at last (u2 §2.10): the user can see there
        // is more, and where. The geometry is the toolkit's — the same
        // arithmetic the host's own views use — and the answer is None
        // when there is nothing to scroll, when `scrollbar.mode = none`,
        // or when the bar has no width.
        //
        // The pointer is asked about the WIDER of the two widths: the
        // bar grows to `scrollbar.w_hover` under the hand, and asking
        // about the narrow one would let it shrink out from under the
        // pointer and flicker at the seam.
        let wide = ScrollbarLook { w: look.bar.w.max(look.bar.w_hover), ..look.bar };
        let hovered = self.view.dragging()
            || scrollbar(area, &wide, offset, viewport, content, false)
                .is_some_and(|g| g.track.contains(mx, my));
        if let Some(g) = scrollbar(area, &look.bar, offset, viewport, content, hovered) {
            // The thumb's rung: idle, hover, or dragging — three of the
            // four the class×state matrix declares for `scrollbar.thumb`,
            // and until the thumb could be taken hold of only the first
            // was ever drawn.
            //
            // `scrollbar.auto_hide` and `scrollbar.fade_ms` are read
            // into the look but NOT applied: honouring them would hide
            // this bar at rest, which is a change to the resting frame
            // this phase must keep identical. `ScrollView::fade_alpha`
            // is the one call it takes, for whoever adopts it once the
            // frame guard is standing.
            let ink = if self.view.dragging() {
                &look.thumb_drag
            } else if hovered {
                &look.thumb_hover
            } else {
                &look.thumb
            };
            let thumb = RectC { x: g.thumb.x, y: g.thumb.y, w: g.thumb.w, h: g.thumb.h };
            if ink.fill.a > 0.0 {
                (api.rect)(ctx, thumb, ink.fill);
            }
            if ink.edge_width > 0.0 && ink.edge.a > 0.0 {
                (api.rect_outline)(ctx, thumb, ink.edge_width, ink.edge);
            }
            self.bar = Some(Bar { viewport, content, track: g.track, thumb: g.thumb });
        }
    }
}

/// Which rows the panel draws, and where the first of them starts —
/// `(first, count, y0)`, with `y0` the top of row `first` relative to
/// the content box and never positive.
///
/// Two answers, and which one is given is the HOST's doing, not the
/// theme's:
///
/// * On a boundary — every frame the master's defaults produce, since
///   `scroll.fling_scale` ships at 0 and a settled view rests on a row
///   — this is the whole-row window the panel has always drawn:
///   `round(scroll / row_h)` rows down, as many as fit whole, nothing
///   sticking out and nothing clipped.
/// * Between two rows, which only a clipping host ever allows, the
///   window includes the rows the edges cut. Drawing those is legal
///   ONLY under `push_clip`, which is why a host without the clip pair
///   never gets this answer: it would paint over the panel below.
fn row_span(
    offset: f32,
    area_h: f32,
    pitch: f32,
    total: usize,
    nvis: usize,
    max_off: usize,
    can_clip: bool,
) -> (usize, usize, f32) {
    let win = virt::row_window(offset, area_h, pitch, total);
    if can_clip && win.y0 < -PARTIAL_ROW {
        return (win.first, win.count, win.y0);
    }
    // `round(scroll / row_h)` — this panel's own arithmetic since it
    // grew a scroll, now living in `view::virt` so the generic views
    // and this one cannot drift apart.
    let first = virt::snap_row(offset, pitch).min(max_off);
    (first, nvis.min(total.saturating_sub(first)), 0.0)
}

/// Trims text (with a trailing ellipsis) so it fits the given width.
fn fit_name(
    api: &HostApi,
    ctx: *mut c_void,
    px: f32,
    text: &str,
    max_w: f32,
    spacing: f32,
) -> String {
    if measure(api, ctx, px, text, spacing) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 1 {
        let cand: String = chars[..n].iter().collect::<String>() + "\u{2026}";
        if measure(api, ctx, px, &cand, spacing) <= max_w {
            return cand;
        }
        n -= 1;
    }
    "\u{2026}".to_string()
}

/// A type role's case transform, applied here because the text entry
/// draws bytes as given. The indices are the schema's declared order —
/// every `*.case` in the master declares `enum: none | upper | lower |
/// smallcaps`, and `theme_enum` indexes that list. Smallcaps needs
/// per-glyph sizes only the host's font system has; through a single
/// text call the nearest honest reading is capitals.
fn recase(word: u32, s: String) -> String {
    match word {
        1 | 3 => s.to_uppercase(), // upper | smallcaps
        2 => s.to_lowercase(),     // lower
        _ => s,                    // none, or a word this build predates
    }
}

/// A filled rectangle with its corners cut off — the toolkit's
/// `chamfer_fill`, as three quads: the middle band and the two
/// trapezoids the cut corners leave.
fn chamfer_fill(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, c: ColorC) {
    let cut = cut.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    (api.rect)(ctx, RectC { x, y: y + cut, w, h: h - 2.0 * cut }, c);
    let top: [f32; 8] = [x + cut, y, x + w - cut, y, x + w, y + cut, x, y + cut];
    (api.quad)(ctx, top.as_ptr(), c);
    let bottom: [f32; 8] =
        [x, y + h - cut, x + w, y + h - cut, x + w - cut, y + h, x + cut, y + h];
    (api.quad)(ctx, bottom.as_ptr(), c);
}

/// The eight points of the chamfer outline, flat, clockwise from the
/// top-left cut. `cut = 0` collapses each corner's pair to one point.
fn octagon(r: RectC, cut: f32) -> [f32; 16] {
    let cut = cut.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    [
        x + cut, y,
        x + w - cut, y,
        x + w, y + cut,
        x + w, y + h - cut,
        x + w - cut, y + h,
        x + cut, y + h,
        x, y + h - cut,
        x, y + cut,
    ]
}

/// The ring of the same shape — a closed polyline through eight points.
fn chamfer_frame(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, t: f32, c: ColorC) {
    let pts = octagon(r, cut);
    (api.polyline)(ctx, pts.as_ptr(), 8, t, c, true);
}

/// Glow OUTSIDE the ring — `DrawList::glow_ring`'s technique reached
/// through ABI 6's `mask_quad`: the outline extruded outward by `radius`,
/// one additive quad per segment, the soft disk's 2-texel cardinal strip
/// laid across the extrusion (u pinned to the stretchable middle, v from
/// the disk's peak on the path to the sprite's zero at the outer rim).
/// The outer path is the chamfer octagon of the grown rect with the cut
/// grown by the same radius — `Corner::inset(-radius)`, as the host grows
/// it — so a chamfered corner glows along its diagonal and a square
/// corner (`cut = 0`: the inner pair collapses to one point) mitres.
/// Nothing is emitted inside the path, so the glow never tints the fill.
fn chamfer_glow(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, radius: f32, c: ColorC) {
    if !(radius > 0.0) || c.a <= 0.0 {
        return;
    }
    let inner = octagon(r, cut);
    let grown = RectC {
        x: r.x - radius,
        y: r.y - radius,
        w: r.w + 2.0 * radius,
        h: r.h + 2.0 * radius,
    };
    let outer = octagon(grown, cut + radius);
    // The strip's profile in the SPRITE's own space: the mask-band
    // contract's 31..33 stretchable middle (r1 §4.2), the same numbers
    // `glow_ring` maps into the atlas band.
    const SU: f32 = 32.0 / 64.0;
    const VI: f32 = 31.0 / 64.0;
    let uv: [f32; 8] = [SU, VI, SU, VI, SU, 0.0, SU, 0.0];
    for i in 0..8 {
        let j = (i + 1) % 8;
        let pts: [f32; 8] = [
            inner[2 * i], inner[2 * i + 1],
            inner[2 * j], inner[2 * j + 1],
            outer[2 * j], outer[2 * j + 1],
            outer[2 * i], outer[2 * i + 1],
        ];
        (api.mask_quad)(ctx, pts.as_ptr(), uv.as_ptr(), c, MASK_QUAD_ADD);
    }
}

fn draw_folder_icon(api: &HostApi, ctx: *mut c_void, r: Rect, c: ColorC, stroke: f32) {
    // Folder: tab + body. The proportions are the glyph itself — what
    // icon.folder.layers replaces wholesale, not what a theme tunes.
    let tab_w = r.w * 0.4;
    let tab_h = r.h * 0.18;
    let pts = [
        [r.x, r.y + tab_h],
        [r.x, r.y],
        [r.x + tab_w, r.y],
        [r.x + tab_w + tab_h, r.y + tab_h],
        [r.right(), r.y + tab_h],
        [r.right(), r.bottom()],
        [r.x, r.bottom()],
    ];
    let flat: Vec<f32> = pts.iter().flat_map(|p| [p[0], p[1]]).collect();
    (api.polyline)(ctx, flat.as_ptr(), pts.len() as u32, stroke, c, true);
}

fn draw_file_icon(
    api: &HostApi,
    ctx: *mut c_void,
    r: Rect,
    c: ColorC,
    detail: ColorC,
    stroke: f32,
    detail_stroke: f32,
) {
    // Sheet with a folded corner; the fold lines are the icon's detail
    // strokes and draw in their own colour and weight.
    let fold = r.w * 0.3;
    let x = r.x + r.w * 0.15;
    let w = r.w * 0.7;
    let pts = [
        [x, r.y],
        [x + w - fold, r.y],
        [x + w, r.y + fold],
        [x + w, r.bottom()],
        [x, r.bottom()],
    ];
    let flat: Vec<f32> = pts.iter().flat_map(|p| [p[0], p[1]]).collect();
    (api.polyline)(ctx, flat.as_ptr(), pts.len() as u32, stroke, c, true);
    (api.line)(
        ctx,
        x + w - fold,
        r.y,
        x + w - fold,
        r.y + fold,
        detail_stroke,
        detail,
    );
    (api.line)(
        ctx,
        x + w - fold,
        r.y + fold,
        x + w,
        r.y + fold,
        detail_stroke,
        detail,
    );
}



// ----------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    let start = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    Box::into_raw(Box::new(Filesystem::new(start))) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Filesystem) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Filesystem> {
    unsafe { (instance as *mut Filesystem).as_mut() }
}

extern "C" fn draw_c(
    instance: *mut c_void,
    ctx: *mut c_void,
    host_data: *const c_void,
    r: RectC,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    // Follow the active shell before drawing, the way this panel always
    // has: a cd typed in the terminal moves the panel with it.
    let mut buf = [0u8; 4096];
    let n = (api.shell_cwd)(host_data, buf.as_mut_ptr(), buf.len() as u32) as usize;
    if n > 0 {
        if let Ok(path) = std::str::from_utf8(&buf[..n]) {
            this.follow(Some(PathBuf::from(path)));
        }
    }
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
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return;
    };
    // The path is kept here until the next click; the host copies it out
    // before returning, so one buffer is enough and its lifetime is
    // something this side can actually promise.
    match this.click(x, y) {
        Some(FsEvent::OpenDir(p)) => {
            this.last_path = p.display().to_string().into_bytes();
            out.kind = ACTION_OPEN_DIR;
            out.data = this.last_path.as_ptr();
            out.data_len = this.last_path.len() as u32;
        }
        Some(FsEvent::OpenFile(p)) => {
            this.last_path = p.display().to_string().into_bytes();
            out.kind = ACTION_OPEN_FILE;
            out.data = this.last_path.as_ptr();
            out.data_len = this.last_path.len() as u32;
        }
        None => out.kind = ACTION_NONE,
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
        // The distance and the physics are the ones the last draw
        // cached — a wheel event arrives with no drawing context to ask
        // the theme through.
        this.wheel(dy);
    }
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = ACTION_NONE;
    }
}

extern "C" fn grid_c(_: *mut c_void, _: *mut u32, _: *mut u32) {}

extern "C" fn key_feedback_c(_: *mut c_void, _: u32, _: *const u8, _: u32) {}

/// Grows downwards: a taller panel is more rows of files, not bigger
/// ones. The width is what decides how big an icon is.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_ROWS
}

/// The header, as chrome: `FILESYSTEM` and the cwd move to the HOST's
/// title band (u2 §2.10 item 1, §4.3) — same strings, same data. The
/// shell is followed here as well as in draw, so the band never shows
/// last frame's directory; the band side does the left trim, in the one
/// place that draws it.
extern "C" fn chrome_c(
    instance: *mut c_void,
    _ctx: *mut c_void,
    host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    static TITLE: &[u8] = b"FILESYSTEM";
    let (Some(api), Some(this), Some(out)) =
        (host(), state(instance), unsafe { out.as_mut() })
    else {
        return 0;
    };
    let mut buf = [0u8; 4096];
    let n = (api.shell_cwd)(host_data, buf.as_mut_ptr(), buf.len() as u32) as usize;
    if n > 0 {
        if let Ok(path) = std::str::from_utf8(&buf[..n]) {
            this.follow(Some(PathBuf::from(path)));
        }
    }
    this.chrome_right = format!("{}", this.cwd.display()).into_bytes();
    out.title = TITLE.as_ptr();
    out.title_len = TITLE.len() as u32;
    out.right = this.chrome_right.as_ptr();
    out.right_len = this.chrome_right.len() as u32;
    (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
}

/// The pointer's whole gesture — the host's single capture path, and
/// what this widget's scroll thumb is dragged by.
///
/// A `Begin` anywhere but on the bar is DECLINED (`ACTION_NONE`), which
/// leaves the press on the ordinary click path: that is how a tile is
/// still opened by releasing on it. A `Begin` on the bar answers
/// `ACTION_CAPTURE` — the gesture is the widget's and the application
/// is asked for nothing — and the host then routes every motion here
/// and no click at the end.
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
                kind = if this.press(x, y) { ACTION_CAPTURE } else { ACTION_NONE };
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
/// `filesystem.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `filesystem.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "filesystem",
    meta: include_str!("../filesystem.meta"),
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
/// Called by the host with its own interface, once, before anything else.
/// `api` must point at a `HostApi` the host keeps alive for the life of
/// the program.
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
mod tests {
    use super::*;

    /// A panel of 40 rows of 100 px seen through 250: nvis = 2 whole
    /// rows on a 250-px box, max offset 38 rows.
    const PITCH: f32 = 100.0;
    const AREA_H: f32 = 250.0;
    const TOTAL: usize = 40;
    const NVIS: usize = 2;
    const MAX_OFF: usize = TOTAL - NVIS;

    /// A widget with no directory behind it: the constructor's read
    /// fails, the entry list stays empty, and every test below is about
    /// the scroll and nothing else. No test of this file touches a
    /// directory that exists.
    fn widget() -> Filesystem {
        Filesystem::new(PathBuf::from("/nonexistent/nacelle-scroll-test"))
    }

    fn bar() -> Bar {
        Bar {
            viewport: NVIS as f32 * PITCH,
            content: TOTAL as f32 * PITCH,
            track: Rect::new(90.0, 0.0, 6.0, 100.0),
            thumb: Rect::new(90.0, 0.0, 6.0, 20.0),
        }
    }

    #[test]
    fn a_tile_the_edge_cuts_is_clickable_only_where_it_shows() {
        let area = Rect::new(0.0, 100.0, 200.0, 300.0);
        // Wholly inside: the hit rectangle is the tile itself.
        let inside = Rect::new(10.0, 110.0, 50.0, 50.0);
        let m = meet(inside, area);
        assert_eq!((m.x, m.y, m.w, m.h), (10.0, 110.0, 50.0, 50.0));
        // Half above the content box: only the half below the edge.
        let cut = Rect::new(10.0, 80.0, 50.0, 50.0);
        let m = meet(cut, area);
        assert_eq!((m.y, m.h), (100.0, 30.0));
        // Wholly above it: nothing, and nothing is pushed as a hit.
        let gone = meet(Rect::new(10.0, 0.0, 50.0, 50.0), area);
        assert_eq!(gone.h, 0.0);
    }

    #[test]
    fn a_settled_view_draws_the_whole_rows_it_always_did() {
        // On a boundary, with or without a clip, the answer is the one
        // the panel has always given: the row the offset rounds to, as
        // many whole rows as fit, and no fraction anywhere.
        for can_clip in [false, true] {
            let (first, count, y0) =
                row_span(3.0 * PITCH, AREA_H, PITCH, TOTAL, NVIS, MAX_OFF, can_clip);
            assert_eq!((first, count, y0), (3, NVIS, 0.0), "clip = {can_clip}");
        }
        // The top and the very end of the travel, likewise.
        assert_eq!(row_span(0.0, AREA_H, PITCH, TOTAL, NVIS, MAX_OFF, true).0, 0);
        let end = row_span(MAX_OFF as f32 * PITCH, AREA_H, PITCH, TOTAL, NVIS, MAX_OFF, true);
        assert_eq!((end.0, end.1, end.2), (MAX_OFF, NVIS, 0.0));
    }

    #[test]
    fn a_host_that_cannot_clip_never_gets_a_cut_row() {
        // Mid-row, the clipping host draws the rows the edges cut and
        // says where the first one starts...
        let (first, count, y0) =
            row_span(3.5 * PITCH, AREA_H, PITCH, TOTAL, NVIS, MAX_OFF, true);
        assert_eq!(first, 3);
        assert!(y0 < 0.0, "the first row starts above the box");
        assert!(count > NVIS, "the row the bottom edge cuts is drawn too");
        // ... and the host without the clip pair rounds to a row, which
        // is this panel's behaviour on every host it has ever run on.
        let (first, count, y0) =
            row_span(3.5 * PITCH, AREA_H, PITCH, TOTAL, NVIS, MAX_OFF, false);
        assert_eq!((first, count, y0), (4, NVIS, 0.0));
    }

    #[test]
    fn the_window_never_asks_for_a_row_the_model_has_not_got() {
        // An empty directory, and one shorter than the viewport.
        assert_eq!(row_span(0.0, AREA_H, PITCH, 0, NVIS, 0, true).1, 0);
        assert_eq!(row_span(0.0, AREA_H, PITCH, 1, NVIS, 0, true).1, 1);
        // A viewport of nothing, mid-resize.
        assert_eq!(row_span(0.0, 0.0, PITCH, TOTAL, 1, 39, true).1, 1);
    }

    #[test]
    fn a_notch_moves_one_wheel_px_and_lands() {
        let mut fs = widget();
        fs.physics = ScrollPhysics {
            wheel_px: 43.0,
            fling_scale: 0.0,
            glide_halflife_ms: 160.0,
            settle_ms: 220.0,
            settle_easing: Easing::EaseOut,
            motion_scale: 1.0,
        };
        // The host spells "toward the top" as a positive delta, and the
        // offset counts pixels from the top: the sign flips exactly
        // once, and a wheel at the top of the list goes nowhere.
        fs.wheel(-1.0);
        assert_eq!(fs.view.offset(), 43.0);
        fs.wheel(1.0);
        assert_eq!(fs.view.offset(), 0.0);
        // Kinetics is off in the master: a notch is a move, not a
        // flick, and nothing is left in flight to glide.
        assert_eq!(fs.view.velocity(), 0.0);
    }

    #[test]
    fn only_the_bar_takes_a_press() {
        let mut fs = widget();
        // No bar drawn last frame: nothing is taken, and the press
        // stays on the click path where a tile is opened.
        assert!(!fs.press(92.0, 10.0));
        fs.bar = Some(bar());
        // Beside the bar, over the tiles: still not ours.
        assert!(!fs.press(10.0, 10.0));
        assert!(!fs.view.dragging());
    }

    #[test]
    fn the_thumb_follows_the_hand_and_settles_when_it_is_let_go() {
        let mut fs = widget();
        fs.bar = Some(bar());
        // Taken hold of 10 px down the thumb.
        assert!(fs.press(92.0, 10.0));
        assert!(fs.view.dragging());
        // The hand moves halfway down the track; the thumb goes where
        // the hand is, and the offset with it. Travel is 80 px of track
        // for 3800 px of content.
        fs.drag_to(50.0);
        assert!((fs.view.offset() - 40.0 / 80.0 * 3800.0).abs() < 0.5);
        fs.release();
        assert!(!fs.view.dragging());
    }

    #[test]
    fn a_press_beside_the_thumb_pages_and_is_still_ours() {
        let mut fs = widget();
        fs.bar = Some(bar());
        // Below the thumb: one viewport toward the end. The press is
        // taken even though nothing is grabbed — the overlay bar sits
        // ON TOP of the tiles, and letting it through would open a file
        // the hand never aimed at.
        assert!(fs.press(92.0, 60.0));
        assert!(!fs.view.dragging());
        assert_eq!(fs.view.offset(), NVIS as f32 * PITCH);
        // The next frame draws the thumb further down the track; a
        // press ABOVE it pages back the way it came.
        fs.bar = Some(Bar { thumb: Rect::new(90.0, 40.0, 6.0, 20.0), ..bar() });
        assert!(fs.press(92.0, 10.0));
        assert_eq!(fs.view.offset(), 0.0);
    }
}
