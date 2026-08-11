//! The tile grid itself — the part of this widget that is not "which
//! list": the theme it reads, the arithmetic that turns a content box
//! into rows of tiles, and the shapes one tile is made of.
//!
//! It lives in a module of its own because it has a second caller. The
//! categories widget (`nacelle-widget-appcats`) shows the SAME tiles
//! over a filtered list, and a grid drawn twice from two copies of this
//! arithmetic would be two grids that drift apart on the first token
//! that moves. One system, one source of truth — the same reason the
//! XDG scanner sits in [`crate::desktop`] rather than in either widget.
//!
//! Nothing here holds state and nothing here decides anything: every
//! colour, length and word arrives from the theme through ABI 5/6
//! tokens, and a missing token degrades through the raw answers the ABI
//! itself gives (grey ink, zero lengths), never through a number that
//! used to be the design.

use nacelle::runtime::{ColorC, HostApi, RectC, StateStyleC, MASK_QUAD_ADD};
use std::ffi::c_void;

/// The font slots, as the host numbers them — the theme's own
/// `FACE_UI = 0` and `FACE_MONO = 1`. The ABI carries these two and
/// clamps anything past them, so a slot is chosen by the WORD a role's
/// `face` names and never by an index into the theme's eight faces.
pub const FONT_UI: u32 = 0;
pub const FONT_MONO: u32 = 1;

// The interaction states, as indices into the matrix's declaration
// order (idle, hover, press, selected, selected_hover, dragging,
// disabled). A tile is a container: every one rests on its class's idle
// rung, the pointed-at one on hover, and the just-clicked one on press.
//
// The two `selected` rungs are the categories list's: a row that is
// steering the launcher grid is the chosen one of a set, persistently,
// which is the state the matrix's own comment describes as "this one of
// a set is the chosen one". `selected_hover` exists in the matrix
// because "chosen AND pointed at" really happens in a list, so it is
// asked for rather than approximated from the other two.
pub const STATE_IDLE: u32 = 0;
pub const STATE_HOVER: u32 = 1;
pub const STATE_PRESS: u32 = 2;
pub const STATE_SELECTED: u32 = 3;
pub const STATE_SELECTED_HOVER: u32 = 4;

/// `filetile.row_justify` declares `pack | fill`; the baked enum is the
/// word's index in that list.
pub const ROW_JUSTIFY_FILL: u32 = 1;

/// The engine's raw ink — what `theme_color` answers for a missing
/// token. Kept only for the path where the host predates ABI 5 and
/// cannot be asked at all.
pub const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
pub const NO_COLOR: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// A rectangle in a widget's own arithmetic; `RectC` is what crosses.
#[derive(Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    /// The same rectangle, as the ABI spells it.
    pub fn c(&self) -> RectC {
        RectC { x: self.x, y: self.y, w: self.w, h: self.h }
    }
}

// ------------------------------------------------------------------ theme

pub fn token(api: &HostApi, name: &str) -> u32 {
    (api.theme_token)(name.as_ptr(), name.len() as u32)
}

/// Token ids the tile grid draws from, resolved by NAME once per epoch.
///
/// No header tokens: a panel's title band is the HOST's, through
/// `chrome`. A grid's drawing starts at the content box's first row of
/// tiles.
///
/// Two families meet here, and neither is this file's invention. The
/// GEOMETRY is `filetile.*` — the tile grid's own group, which the
/// theme documents as serving the launcher and the file browser alike
/// ("a launcher tile, a file tile" is how `class.tile` puts it). The
/// TYPE is `type.caption`, the role the theme describes in as many
/// words as "a launcher tile caption, FILES". Where a token for the
/// launcher grid does not exist at all, the file browser's is used and
/// said so in the report, never replaced by a number.
pub struct TileTheme {
    pub epoch: u32,
    // form — the tile grid
    pub gap: u32,      // filetile.gap
    pub rows: u32,     // filetile.rows
    pub cols: u32,     // filetile.cols — a count, or the `auto` sentinel
    pub cell_min: u32, // filetile.cell_min_px
    pub cell_pref: u32, // filetile.cell — preferred tile edge; 0 = size from rows
    pub corner: u32,   // filetile.corner — the tile container's chamfer cut
    pub caption_gap: u32, // filetile.caption_gap
    pub icon_inset_x: u32, // filetile.icon.inset_x
    pub icon_inset_y: u32, // filetile.icon.inset_y
    pub icon_w: u32,   // filetile.icon.w
    pub icon_h: u32,   // filetile.icon.h
    pub wheel: u32,    // filetile.wheel_px
    pub row_justify: u32, // filetile.row_justify
    // the stand-in for the icon nobody can draw yet
    pub glyph_px: u32, // icon.size.launcher — "the launcher grid's app glyphs"
    // type — the launcher tile caption role
    pub caption_size: u32,     // type.caption.size
    pub caption_min: u32,      // type.caption.min_px
    pub caption_tracking: u32, // type.caption.tracking
    pub caption_leading: u32,  // type.caption.leading
    pub caption_case: u32,     // type.caption.case
    // where an empty grid says so
    pub empty_y: u32, // emptystate.y_frac
    // the press flash's life, and the one global that scales it
    pub press_ms: u32,     // motion.press.duration_ms
    pub motion_scale: u32, // motion.scale
    pub glow_scale: u32,   // glow.alpha_scale
    // the scrollbar
    pub sb_mode: u32,      // scrollbar.mode — `overlay | inset | none`; overlay = 0
    pub sb_w: u32,         // scrollbar.w
    pub sb_margin: u32,    // scrollbar.margin
    pub sb_thumb_min: u32, // scrollbar.thumb_min
    pub sb_side: u32,      // scrollbar.edge — `right | left`; right = 0
    /// The launcher tile's row in the class x state matrix.
    pub tile_class: u32,
    /// The scroll thumb's row in the same matrix.
    pub thumb_class: u32,
}

impl TileTheme {
    pub fn resolve(api: &HostApi, epoch: u32) -> TileTheme {
        let class = |name: &str| (api.theme_class)(name.as_ptr(), name.len() as u32);
        TileTheme {
            epoch,
            gap: token(api, "filetile.gap"),
            rows: token(api, "filetile.rows"),
            cols: token(api, "filetile.cols"),
            cell_min: token(api, "filetile.cell_min_px"),
            cell_pref: token(api, "filetile.cell"),
            corner: token(api, "filetile.corner"),
            caption_gap: token(api, "filetile.caption_gap"),
            icon_inset_x: token(api, "filetile.icon.inset_x"),
            icon_inset_y: token(api, "filetile.icon.inset_y"),
            icon_w: token(api, "filetile.icon.w"),
            icon_h: token(api, "filetile.icon.h"),
            wheel: token(api, "filetile.wheel_px"),
            row_justify: token(api, "filetile.row_justify"),
            glyph_px: token(api, "icon.size.launcher"),
            caption_size: token(api, "type.caption.size"),
            caption_min: token(api, "type.caption.min_px"),
            caption_tracking: token(api, "type.caption.tracking"),
            caption_leading: token(api, "type.caption.leading"),
            caption_case: token(api, "type.caption.case"),
            empty_y: token(api, "emptystate.y_frac"),
            press_ms: token(api, "motion.press.duration_ms"),
            motion_scale: token(api, "motion.scale"),
            glow_scale: token(api, "glow.alpha_scale"),
            sb_mode: token(api, "scrollbar.mode"),
            sb_w: token(api, "scrollbar.w"),
            sb_margin: token(api, "scrollbar.margin"),
            sb_thumb_min: token(api, "scrollbar.thumb_min"),
            sb_side: token(api, "scrollbar.edge"),
            // `tile` IS the launcher tile's class — the matrix names it
            // "a launcher tile, a file tile, an ADD WIDGET entry" — so
            // this grid takes it directly rather than borrowing the file
            // browser's `filetile` row.
            tile_class: class("tile"),
            thumb_class: class("scrollbar.thumb"),
        }
    }
}

/// The values one frame draws with, read fresh from the resolved ids.
/// Colours and lengths only — nothing here is arithmetic on anything.
pub struct TileLook {
    pub idle: StateStyleC,
    pub hover: StateStyleC,
    pub press: StateStyleC,
    pub thumb: StateStyleC,
    pub gap: f32,
    pub rows: f32,
    pub cols: f32,
    pub cell_min: f32,
    pub cell_pref: f32,
    pub corner: f32,
    pub caption_gap: f32,
    pub icon_inset_x: f32,
    pub icon_inset_y: f32,
    pub icon_w: f32,
    pub icon_h: f32,
    pub wheel_px: f32,
    pub row_justify: u32,
    pub glyph_px: f32,
    pub caption_px: f32,
    pub caption_tracking: f32,
    pub caption_leading: f32,
    pub caption_case: u32,
    pub empty_y: f32,
    /// `motion.press.duration_ms` already scaled by `motion.scale` and
    /// turned into seconds — a reduced-motion theme sets the scale to 0
    /// and the flash simply never shows.
    pub press_s: f32,
    pub glow_scale: f32,
    pub sb_mode: u32,
    pub sb_w: f32,
    pub sb_margin: f32,
    pub sb_thumb_min: f32,
    pub sb_side: u32,
}

impl TileLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// Grey ink, zero lengths — the engine's own defaults, mirrored, so
    /// an old host shows the same undesigned raw as an empty theme.
    pub fn raw() -> TileLook {
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
        TileLook {
            idle: raw_state,
            hover: raw_state,
            press: raw_state,
            thumb: raw_state,
            gap: 0.0,
            rows: 0.0,
            cols: 0.0,
            cell_min: 0.0,
            cell_pref: 0.0,
            corner: 0.0,
            caption_gap: 0.0,
            icon_inset_x: 0.0,
            icon_inset_y: 0.0,
            icon_w: 0.0,
            icon_h: 0.0,
            wheel_px: 0.0,
            row_justify: 0,
            glyph_px: 0.0,
            caption_px: 0.0,
            caption_tracking: 0.0,
            caption_leading: 1.0,
            caption_case: 0,
            empty_y: 0.0,
            press_s: 0.0,
            glow_scale: 0.0,
            sb_mode: 0,
            sb_w: 0.0,
            sb_margin: 0.0,
            sb_thumb_min: 0.0,
            sb_side: 0,
        }
    }

    pub fn read(api: &HostApi, ctx: *mut c_void, t: &TileTheme) -> TileLook {
        let px = |id| (api.theme_px)(ctx, id);
        TileLook {
            idle: rung(api, ctx, t.tile_class, STATE_IDLE),
            hover: rung(api, ctx, t.tile_class, STATE_HOVER),
            press: rung(api, ctx, t.tile_class, STATE_PRESS),
            thumb: rung(api, ctx, t.thumb_class, STATE_IDLE),
            gap: px(t.gap),
            rows: px(t.rows),
            cols: px(t.cols),
            cell_min: px(t.cell_min),
            cell_pref: px(t.cell_pref),
            corner: px(t.corner),
            caption_gap: px(t.caption_gap),
            icon_inset_x: px(t.icon_inset_x),
            icon_inset_y: px(t.icon_inset_y),
            icon_w: px(t.icon_w),
            icon_h: px(t.icon_h),
            wheel_px: px(t.wheel),
            row_justify: (api.theme_enum)(ctx, t.row_justify),
            glyph_px: px(t.glyph_px),
            caption_px: px(t.caption_size).max(px(t.caption_min)),
            caption_tracking: px(t.caption_tracking),
            caption_leading: px(t.caption_leading).max(1.0),
            caption_case: (api.theme_enum)(ctx, t.caption_case),
            empty_y: px(t.empty_y),
            press_s: px(t.press_ms) * px(t.motion_scale) / 1000.0,
            glow_scale: px(t.glow_scale),
            sb_mode: (api.theme_enum)(ctx, t.sb_mode),
            sb_w: px(t.sb_w),
            sb_margin: px(t.sb_margin),
            sb_thumb_min: px(t.sb_thumb_min),
            sb_side: (api.theme_enum)(ctx, t.sb_side),
        }
    }
}

/// A rung of a class's ladder, whole. A missing class answers the
/// matrix's own raw rung, so no fallback lives here.
pub fn rung(api: &HostApi, ctx: *mut c_void, class: u32, state: u32) -> StateStyleC {
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
    (api.theme_class_state)(ctx, class, state, &mut out, std::mem::size_of::<StateStyleC>() as u32);
    out
}

// ------------------------------------------------------------------- text

pub fn measure(
    api: &HostApi,
    ctx: *mut c_void,
    font: u32,
    px: f32,
    text: &str,
    spacing: f32,
) -> f32 {
    (api.measure)(ctx, font, px, text.as_ptr(), text.len() as u32, spacing)
}

/// One run of text. `align` is the host's: 0 left, 1 centre, 2 right.
#[allow(clippy::too_many_arguments)]
pub fn text(
    api: &HostApi,
    ctx: *mut c_void,
    font: u32,
    px: f32,
    x: f32,
    y: f32,
    s: &str,
    c: ColorC,
    spacing: f32,
    align: u32,
) {
    (api.text)(ctx, font, px, x, y, s.as_ptr(), s.len() as u32, c, spacing, align);
}

/// A run centred on `x` in the interface font — what a tile's caption
/// and glyph both want.
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    api: &HostApi,
    ctx: *mut c_void,
    px: f32,
    x: f32,
    y: f32,
    s: &str,
    c: ColorC,
    spacing: f32,
) {
    text(api, ctx, FONT_UI, px, x, y, s, c, spacing, 1);
}

/// The font slot a type role's `face` token names.
///
/// A face is an OPEN word set — `ui`, `mono`, `ui_bold`, `display` and
/// `icon` are all faces the theme declares — so it is read as a WORD
/// rather than as an index: the boundary numbers two slots and clamps
/// anything past them, which would turn `display` into monospace. A
/// mono face answers the mono slot; every other face answers the
/// interface slot, which is where the boundary puts them all anyway.
///
/// Init-time work, like [`token`]: this copies a string, so it belongs
/// beside the id resolution and behind the epoch, never in a frame.
pub fn face_slot(api: &HostApi, ctx: *mut c_void, id: u32) -> u32 {
    if !api.has_theme_enum_word() || id == u32::MAX {
        return FONT_UI;
    }
    let mut buf = [0u8; 32];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    let word = std::str::from_utf8(&buf[..n.min(buf.len())]).unwrap_or("");
    if word.starts_with("mono") {
        FONT_MONO
    } else {
        FONT_UI
    }
}

/// The one character that stands in for an application's icon: the
/// first of its name, as a capital. A name that begins with something
/// uncased (a digit, a CJK ideograph) is left as it is, which is what
/// uppercasing means for those scripts anyway.
pub fn initial(name: &str) -> String {
    match name.chars().next() {
        Some(c) => c.to_uppercase().collect(),
        None => String::new(),
    }
}

/// Trims text (with a trailing ellipsis) so it fits the given width.
#[allow(clippy::too_many_arguments)]
pub fn fit_name(
    api: &HostApi,
    ctx: *mut c_void,
    font: u32,
    px: f32,
    text: &str,
    max_w: f32,
    spacing: f32,
) -> String {
    if measure(api, ctx, font, px, text, spacing) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 1 {
        let cand: String = chars[..n].iter().collect::<String>() + "\u{2026}";
        if measure(api, ctx, font, px, &cand, spacing) <= max_w {
            return cand;
        }
        n -= 1;
    }
    "\u{2026}".to_string()
}

/// A type role's case transform, applied here because the text entry
/// draws bytes as given. The indices are the schema's declared order —
/// every `*.case` declares `enum: none | upper | lower | smallcaps`,
/// and `theme_enum` indexes that list. Smallcaps needs per-glyph sizes
/// only the host's font system has; through a single text call the
/// nearest honest reading is capitals.
pub fn recase(word: u32, s: String) -> String {
    match word {
        1 | 3 => s.to_uppercase(), // upper | smallcaps
        2 => s.to_lowercase(),     // lower
        _ => s,                    // none, or a word this build predates
    }
}

// ----------------------------------------------------------------- shapes

/// One container's whole surface on one rung of the ladder: the fill,
/// the ring, and the ring's glow — in that order, chamfered by `cut`.
/// This is what makes a tile look like a tile; a caller that wants the
/// same look on a different shape passes a different rectangle, never a
/// different colour.
pub fn frame(
    api: &HostApi,
    ctx: *mut c_void,
    cell: RectC,
    cut: f32,
    rung: &StateStyleC,
    glow_scale: f32,
) {
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
        // Right after the stroke, the ring's glow — this rung's own
        // `glow_radius` and `glow_alpha` from the ladder, tinted with
        // the edge's resolved colour and scaled by the one global knob.
        // Every shipped idle rung is dark; hover and press are where the
        // ladder lights up.
        let alpha = (rung.glow_alpha * glow_scale).clamp(0.0, 1.0);
        if api.has_mask_quad() && rung.glow_radius > 0.0 && alpha > 0.0 {
            let c = ColorC { a: alpha, ..rung.edge };
            chamfer_glow(api, ctx, cell, cut, rung.glow_radius, c);
        }
    }
}

/// A filled rectangle with its corners cut off — the toolkit's
/// `chamfer_fill`, as three quads: the middle band and the two
/// trapezoids the cut corners leave.
pub fn chamfer_fill(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, c: ColorC) {
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
pub fn octagon(r: RectC, cut: f32) -> [f32; 16] {
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
pub fn chamfer_frame(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, t: f32, c: ColorC) {
    let pts = octagon(r, cut);
    (api.polyline)(ctx, pts.as_ptr(), 8, t, c, true);
}

/// Glow OUTSIDE the ring — the outline extruded outward by `radius`,
/// one additive quad per segment, the soft disk's cardinal strip laid
/// across the extrusion. Nothing is emitted inside the path, so the
/// glow never tints the fill.
pub fn chamfer_glow(
    api: &HostApi,
    ctx: *mut c_void,
    r: RectC,
    cut: f32,
    radius: f32,
    c: ColorC,
) {
    // A theme can hand back anything, NaN included; the comparison is
    // written so that "not a usable radius" covers it.
    if !radius.is_finite() || radius <= 0.0 || c.a <= 0.0 {
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
    // contract's 31..33 stretchable middle.
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

// ----------------------------------------------------------------- layout

/// One content box's worth of grid: how big a tile is, how many fit,
/// which rows are on screen. Everything a caller needs to place item
/// `i` and to draw the bar that says where it is.
pub struct Layout {
    pub tile: f32,
    pub gap: f32,
    pub cols: usize,
    pub total_rows: usize,
    /// Rows that fit the box whole.
    pub nvis: usize,
    /// The furthest first-row this list can be scrolled to.
    pub max_off: usize,
    /// The first row on screen.
    pub row_off: usize,
    /// Vertical pitch between two rows, which `filetile.row_justify`
    /// may stretch past `tile + gap`.
    pub step: f32,
}

/// How big one tile is in `area`, and how many fit across it.
///
/// The sizing half of [`layout`], on its own because the alphabetical
/// index ([`crate::sections`]) lays the SAME tiles out down a column
/// broken by headings: same edge, same columns, a different vertical
/// program. Two copies of this arithmetic would be two tile sizes that
/// disagree the moment the panel is resized, which is the same reason
/// this module exists at all.
pub fn cells(look: &TileLook, area: Rect) -> (f32, usize) {
    let gap = look.gap;
    let rows_page = look.rows.max(1.0);
    // filetile.cell names the tile edge directly; rows-per-page is
    // the fallback for a theme that sizes by page count instead.
    let row_cell = if look.cell_pref > 0.0 {
        look.cell_pref.max(look.cell_min)
    } else {
        ((area.h - gap * (rows_page - 1.0)) / rows_page).max(look.cell_min)
    };
    // filetile.cols: a count, or the `auto` sentinel (< 1), which
    // fits as many row-sized cells as the width allows.
    let cols = if look.cols >= 1.0 {
        look.cols.round() as usize
    } else if row_cell + gap > 0.0 {
        (((area.w + gap) / (row_cell + gap)).floor() as usize).max(1)
    } else {
        1
    };
    // Never taller than the content box: at least one row is always
    // drawn, so a tile sized only by the width would overrun the
    // panel's bottom edge on a squeezed column and paint over the
    // neighbour below.
    let tile = ((area.w - gap * (cols as f32 - 1.0)) / cols as f32)
        .min(row_cell)
        .min(area.h.max(look.cell_min))
        .max(look.cell_min);
    (tile, cols)
}

/// The grid `count` items make in `area`, and the scroll offset clamped
/// to it. `scroll` is the caller's own state and is corrected here,
/// because the clamp depends on arithmetic only this function does.
pub fn layout(look: &TileLook, area: Rect, count: usize, scroll: &mut f32) -> Layout {
    let gap = look.gap;
    let (tile, cols) = cells(look, area);

    // Scrolling snaps to whole rows — only fully fitting rows are
    // drawn, nothing sticks out of the panel.
    let row_h = tile + gap;
    let total_rows = count.div_ceil(cols);
    let nvis = if row_h > 0.0 {
        (((area.h + gap) / row_h).floor() as usize).max(1)
    } else {
        1
    };
    let max_off = total_rows.saturating_sub(nvis);
    *scroll = scroll.clamp(0.0, (max_off as f32 * row_h).max(0.0));
    let row_off = if row_h > 0.0 {
        ((*scroll / row_h).round() as usize).min(max_off)
    } else {
        0
    };
    // filetile.row_justify = fill stretches the pitch so the last
    // visible row ends exactly at the panel's bottom edge; pack sits
    // every row on filetile.gap.
    let step = if look.row_justify == ROW_JUSTIFY_FILL && total_rows > nvis && nvis > 1 {
        (area.h - tile) / (nvis as f32 - 1.0)
    } else {
        row_h
    };
    Layout { tile, gap, cols, total_rows, nvis, max_off, row_off, step }
}

impl Layout {
    /// Where item `i` sits, or None when its row is off screen.
    pub fn place(&self, area: Rect, i: usize) -> Option<Rect> {
        let col = i % self.cols;
        let row = i / self.cols;
        if row < self.row_off || row >= self.row_off + self.nvis {
            return None;
        }
        Some(Rect::new(
            area.x + col as f32 * (self.tile + self.gap),
            area.y + (row - self.row_off) as f32 * self.step,
            self.tile,
            self.tile,
        ))
    }
}

/// One whole tile: the container on its rung, the mark that stands in
/// for the icon nobody can draw yet, and the caption under it.
///
/// `mark` goes in the box `filetile.icon.*` reserves, never taller than
/// that box, so a squeezed grid shrinks it instead of spilling it over
/// the caption. `label` is recased by the caption role and trimmed by
/// measured width. Both widgets in this crate's family draw their tiles
/// through here, which is what makes "the same tile" a fact rather than
/// a resemblance.
pub fn tile_face(
    api: &HostApi,
    ctx: *mut c_void,
    look: &TileLook,
    t: Rect,
    rung: &StateStyleC,
    mark: &str,
    label: &str,
) {
    let cut = look.corner.min(t.w / 2.0);
    frame(api, ctx, t.c(), cut, rung, look.glow_scale);

    let icon = Rect::new(
        t.x + t.w * look.icon_inset_x,
        t.y + t.h * look.icon_inset_y,
        t.w * look.icon_w,
        t.h * look.icon_h,
    );
    let gpx = look.glyph_px.min(icon.h).max(0.0);
    if !mark.is_empty() {
        draw_text(api, ctx, gpx, icon.cx(), icon.cy() - gpx / 2.0, mark, rung.glyph, 0.0);
    }

    let px = look.caption_px;
    let sp = px * look.caption_tracking;
    let name = recase(look.caption_case, label.to_string());
    let name = fit_name(api, ctx, FONT_UI, px, &name, t.w, sp);
    draw_text(api, ctx, px, t.cx(), t.y + t.w * look.caption_gap, &name, rung.text, sp);
}

/// Where a scrolling column of rows currently is: how many there are,
/// how many are on screen, which is first, and how far it can go. A
/// grid of tiles and a list of rows differ in what a row IS and in
/// nothing else, which is why the bar below takes this and not either
/// widget's own layout.
#[derive(Clone, Copy)]
pub struct Scroll {
    pub total: usize,
    pub nvis: usize,
    pub off: usize,
    pub max_off: usize,
}

impl Layout {
    /// This grid, as the scrollbar reads it.
    pub fn scroll(&self) -> Scroll {
        Scroll {
            total: self.total_rows,
            nvis: self.nvis,
            off: self.row_off,
            max_off: self.max_off,
        }
    }
}

/// The bar that says where in a long list the eye is. Overlay only —
/// the word an enum index can decode across the ABI; any other (inset,
/// none) draws nothing until the ABI can tell them apart.
pub fn scrollbar(api: &HostApi, ctx: *mut c_void, look: &TileLook, area: Rect, s: Scroll) {
    let (total_rows, nvis, row_off, max_off) = (s.total, s.nvis, s.off, s.max_off);
    if !(total_rows > nvis && look.sb_mode == 0 && look.sb_w > 0.0) {
        return;
    }
    let bw = look.sb_w;
    let bx = if look.sb_side == 0 {
        area.right() - look.sb_margin - bw
    } else {
        area.x + look.sb_margin
    };
    let frac = (nvis as f32 / total_rows as f32).clamp(0.0, 1.0);
    let th = (area.h * frac).max(look.sb_thumb_min).min(area.h);
    let ty = area.y + (area.h - th) * (row_off as f32 / max_off.max(1) as f32).clamp(0.0, 1.0);
    let thumb = RectC { x: bx, y: ty, w: bw, h: th };
    if look.thumb.fill.a > 0.0 {
        (api.rect)(ctx, thumb, look.thumb.fill);
    }
    if look.thumb.edge_width > 0.0 && look.thumb.edge.a > 0.0 {
        (api.rect_outline)(ctx, thumb, look.thumb.edge_width, look.thumb.edge);
    }
}
