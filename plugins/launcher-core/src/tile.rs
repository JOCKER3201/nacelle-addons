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
//! tokens, and a token nobody can answer degrades to no ink and no
//! length — nothing drawn — never to a number that used to be the
//! design.

use nacelle::runtime::{
    ColorC, HostApi, RectC, StateStyleC, CORNER_CHAMFER, CORNER_ROUND, CORNER_SQUARE,
    MASK_QUAD_ADD,
};
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

/// No colour at all — what this family draws with on a host that
/// predates ABI 5 and cannot be asked what anything looks like.
///
/// Not a grey: a grey chosen here is a design decision taken where the
/// theme cannot be reached. Paired with the zero widths and zero lengths
/// of [`TileLook::raw`], it makes that host draw NOTHING, which is the
/// clean bail `ai` takes for the same case.
pub const NO_COLOR: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// How a container's corners are CUT, and how far. A radius alone is a
/// length and not a shape — the master's `[corner]` section says so —
/// so the two travel together and neither is decided here.
#[derive(Clone, Copy)]
pub struct Corner {
    /// [`CORNER_SQUARE`], [`CORNER_ROUND`] or [`CORNER_CHAMFER`].
    pub style: u32,
    pub radius: f32,
}

/// The cut a `*_corner_style` word names. A word this build has never
/// heard of leaves the shape square, which is what an unstyled rectangle
/// already is — never a cut of this file's choosing.
pub fn corner_style(word: &str) -> u32 {
    match word {
        "round" => CORNER_ROUND,
        "chamfer" => CORNER_CHAMFER,
        _ => CORNER_SQUARE,
    }
}

/// Whether a scrolling area's bar is drawn at all, and whether it costs
/// layout — `scrollbar.mode`'s three words, decoded from the WORD.
///
/// Read as a word and not as an index: the index-only reading could not
/// tell `inset` from `none`, so a theme that asked for a bar beside the
/// content got no bar at all.
#[derive(Clone, Copy, PartialEq)]
pub enum BarMode {
    Overlay,
    Inset,
    None,
}

pub fn bar_mode(word: &str) -> BarMode {
    match word {
        "none" => BarMode::None,
        // Honoured as far as this grid can: the bar is drawn, but the
        // tiles are laid out before there is a bar to make room for, so
        // it costs no width yet. Said out loud rather than silently
        // drawing nothing, which is what the index reading did.
        "inset" => BarMode::Inset,
        _ => BarMode::Overlay,
    }
}

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

/// The WORD an enum token currently resolves to — ABI 6's appended
/// `theme_enum_word`. Init-time work, like [`token`]: it copies a
/// string, so it belongs beside the id resolution and behind the epoch,
/// never inside a frame. An empty answer — a host whose table ends
/// before the entry, a missing token, a token with no word — is what a
/// caller degrades on.
pub fn enum_word(api: &HostApi, ctx: *mut c_void, id: u32) -> String {
    if !api.has_theme_enum_word() || id == u32::MAX {
        return String::new();
    }
    let mut buf = [0u8; 64];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
}

/// The name of one token of the role a `*_role` binding names, or `None`
/// for a master that binds no role at all — which leaves every id
/// MISSING and every accessor on zero, and type of no size draws
/// nothing. Naming a role here would be this file choosing the type.
pub fn role_token(role: &str, suffix: &str) -> Option<String> {
    if role.is_empty() {
        return None;
    }
    Some(format!("type.{role}.{suffix}"))
}

/// The id of one token of the role a binding names.
pub fn role_id(api: &HostApi, role: &str, suffix: &str) -> u32 {
    match role_token(role, suffix) {
        Some(name) => token(api, &name),
        None => u32::MAX,
    }
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
    // type.<tile.caption_role>.* — the role the master BINDS a launcher
    // tile's caption to. Read as a word: `tile.caption_role` and
    // `filetile.caption_role` are two different words for two grids, and
    // a file that spells `type.caption.*` out reads neither.
    pub caption_size: u32,
    pub caption_min: u32,
    pub caption_tracking: u32,
    pub caption_leading: u32,
    pub caption_case: u32,
    /// The slot that role's `face` names, resolved WITH the ids because
    /// a face is a word and reading words is init-time work.
    pub caption_font: u32,
    // where an empty grid says so
    pub empty_y: u32, // emptystate.y_frac
    // the press flash's life, and the one global that scales it
    pub press_ms: u32,     // motion.press.duration_ms
    pub motion_scale: u32, // motion.scale
    pub glow_scale: u32,   // glow.alpha_scale
    // the scrollbar
    /// `scrollbar.mode`, decoded from its WORD beside the ids.
    pub sb_mode: BarMode,
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
    pub fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> TileTheme {
        let class = |name: &str| (api.theme_class)(name.as_ptr(), name.len() as u32);
        // The caption's binding, followed to the role it names. An
        // unbound one leaves every id below MISSING, which is a caption
        // of no size: a grid whose type the master says nothing about
        // shows its tiles and no names.
        let caption = enum_word(api, ctx, token(api, "tile.caption_role"));
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
            caption_size: role_id(api, &caption, "size"),
            caption_min: role_id(api, &caption, "min_px"),
            caption_tracking: role_id(api, &caption, "tracking"),
            caption_leading: role_id(api, &caption, "leading"),
            caption_case: role_id(api, &caption, "case"),
            caption_font: face_slot(api, ctx, role_id(api, &caption, "face")),
            empty_y: token(api, "emptystate.y_frac"),
            press_ms: token(api, "motion.press.duration_ms"),
            motion_scale: token(api, "motion.scale"),
            glow_scale: token(api, "glow.alpha_scale"),
            sb_mode: bar_mode(&enum_word(api, ctx, token(api, "scrollbar.mode"))),
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
    pub caption_font: u32,
    pub empty_y: f32,
    /// `motion.press.duration_ms` already scaled by `motion.scale` and
    /// turned into seconds — a reduced-motion theme sets the scale to 0
    /// and the flash simply never shows.
    pub press_s: f32,
    pub glow_scale: f32,
    pub sb_mode: BarMode,
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
            edge: NO_COLOR,
            text: NO_COLOR,
            glyph: NO_COLOR,
            edge_width: 0.0,
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
            caption_font: FONT_UI,
            empty_y: 0.0,
            press_s: 0.0,
            glow_scale: 0.0,
            sb_mode: BarMode::None,
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
            caption_font: t.caption_font,
            empty_y: px(t.empty_y),
            press_s: px(t.press_ms) * px(t.motion_scale) / 1000.0,
            glow_scale: px(t.glow_scale),
            sb_mode: t.sb_mode,
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
/// the ring, and the ring's glow — in that order, on the corners
/// `corner` states. This is what makes a tile look like a tile; a caller
/// that wants the same look on a different shape passes a different
/// [`Corner`], never a different colour.
///
/// The round cut goes through the host's own ring pair, which
/// tessellates the arc by its quarter-pixel rule, so no caller has to
/// know how many segments a radius needs. A host too old for that pair
/// draws the chamfer it always did.
pub fn frame(
    api: &HostApi,
    ctx: *mut c_void,
    cell: RectC,
    corner: Corner,
    rung: &StateStyleC,
    glow_scale: f32,
) {
    // `pill` is a WORD ABOUT THIS BOX, not a length: §5.0 bakes it to a
    // negative sentinel, so clamping the radius at zero — which is what
    // this line did — answered a master writing `@corner.pill` with the
    // very square it wrote to avoid, and said nothing about it. Both
    // doors below this one already read the sentinel (`AbiSurface::
    // ring_fill` on the way out, the host's own `corners_in` on the way
    // in), which left this clamp as the last thing standing between the
    // theme's capsule and a silent rectangle — and the only one on the
    // chamfer path, which never reaches the ring pair at all.
    //
    // The translation is the toolkit's, never repeated here: a capsule
    // written twice is a capsule that stops being one somewhere. It is
    // idempotent, so a caller that resolved its own sentinel first hands
    // in a plain length and gets it back.
    let cut = nacelle::theme::corner_radius(corner.radius, cell.w, cell.h);
    let round = corner.style == CORNER_ROUND && api.has_ring();
    if rung.fill.a > 0.0 {
        if round {
            (api.ring_fill)(ctx, cell, CORNER_ROUND, cut, rung.fill);
        } else if cut > 0.0 && corner.style != CORNER_SQUARE {
            chamfer_fill(api, ctx, cell, cut, rung.fill);
        } else {
            (api.rect)(ctx, cell, rung.fill);
        }
    }
    if rung.edge_width > 0.0 && rung.edge.a > 0.0 {
        if round {
            (api.ring)(ctx, cell, CORNER_ROUND, cut, rung.edge_width, rung.edge);
        } else if cut > 0.0 && corner.style != CORNER_SQUARE {
            chamfer_frame(api, ctx, cell, cut, rung.edge_width, rung.edge);
        } else {
            (api.rect_outline)(ctx, cell, rung.edge_width, rung.edge);
        }
        // Right after the stroke, the ring's glow — this rung's own
        // `glow_radius` and `glow_alpha` from the ladder, tinted with
        // the edge's resolved colour and scaled by the one global knob.
        // Every shipped idle rung is dark; hover and press are where the
        // ladder lights up.
        //
        // The halo is extruded from the octagon whatever the cut is: it
        // is a soft light around the shape, and at a corner radius the
        // difference between an arc's halo and a bevel's is under the
        // sprite's own falloff.
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
    // `filetile.corner` is the radius, and the cut is ROUND because the
    // `[corner]` header states that for every radius with no
    // `*_corner_style` sibling beside it — "a rule of this file rather
    // than a default any drawing code may pick for itself". The header's
    // list of such radii omits this one, but the list is the rule's
    // examples and the condition is "declares no such sibling", which
    // `filetile` does not; the keyboard cap reads the same sentence the
    // same way. The bevel this grid used to wear was the drawing code
    // picking, and the sibling key that would let a theme ask for it
    // back is reported, not invented here.
    let corner = Corner { style: CORNER_ROUND, radius: look.corner.min(t.w / 2.0) };
    frame(api, ctx, t.c(), corner, rung, look.glow_scale);

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
    let name = fit_name(api, ctx, look.caption_font, px, &name, t.w, sp);
    text(
        api,
        ctx,
        look.caption_font,
        px,
        t.cx(),
        t.y + t.w * look.caption_gap,
        &name,
        rung.text,
        sp,
        1,
    );
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

/// The bar that says where in a long list the eye is.
///
/// `scrollbar.mode` decides whether there is one: `none` draws nothing,
/// and `overlay` and `inset` both draw — the difference between them is
/// whether the bar costs the content width, which this grid cannot yet
/// give it (the tiles are laid out before there is a bar). Until the
/// index reading was replaced by the WORD, `inset` and `none` were the
/// same answer here, so a theme asking for a bar beside the tiles got
/// no bar at all.
pub fn scrollbar(api: &HostApi, ctx: *mut c_void, look: &TileLook, area: Rect, s: Scroll) {
    let (total_rows, nvis, row_off, max_off) = (s.total, s.nvis, s.off, s.max_off);
    if !(total_rows > nvis && look.sb_mode != BarMode::None && look.sb_w > 0.0) {
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

#[cfg(test)]
mod token_tests {
    use super::*;

    /// The tile grid's caption is set in the role the master BINDS, and
    /// the binding is followed to a family that really exists — the
    /// chain `TileTheme::resolve` walks. A renamed role fails here
    /// instead of leaving every tile nameless.
    #[test]
    fn the_tile_caption_role_names_a_family_the_master_declares() {
        nacelle::theme::load();
        let id = nacelle::theme::id("tile.caption_role").expect("tile.caption_role");
        let role = nacelle::theme::enum_word_of(id).expect("the binding names no word");
        assert!(!role.is_empty());
        for suffix in ["size", "min_px", "tracking", "leading", "case", "face"] {
            let name = role_token(&role, suffix).expect("a bound role names its family");
            assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
        }
    }

    /// The launcher's caption role and the file browser's are two
    /// different words for two grids — which is why neither could be
    /// spelled into the code. If a master ever binds both to one role
    /// this test says so rather than letting the distinction rot.
    #[test]
    fn the_two_tile_grids_are_bound_to_two_different_roles() {
        nacelle::theme::load();
        let word = |n: &str| {
            nacelle::theme::enum_word_of(nacelle::theme::id(n).expect(n)).expect("no word")
        };
        assert_ne!(word("tile.caption_role"), word("filetile.caption_role"));
    }

    /// THE scrollbar finding: `inset` and `none` are different answers.
    /// Read as an index they were both "not overlay", so a theme asking
    /// for a bar beside the content got no bar at all.
    #[test]
    fn inset_asks_for_a_bar_and_none_asks_for_no_bar() {
        assert!(bar_mode("inset") != BarMode::None);
        assert!(bar_mode("overlay") != BarMode::None);
        assert!(bar_mode("none") == BarMode::None);
        // A word this build predates is a bar, not silence: an unknown
        // arrangement is still an arrangement.
        assert!(bar_mode("") != BarMode::None);
        // And the master's own word is one of the three.
        nacelle::theme::load();
        let id = nacelle::theme::id("scrollbar.mode").expect("scrollbar.mode");
        let word = nacelle::theme::enum_word_of(id).expect("no word");
        assert!(matches!(word.as_str(), "overlay" | "inset" | "none"), "{word}");
    }

    /// A radius is not a shape: the same length is three different
    /// containers depending on the word beside it, and a word this file
    /// has never heard of leaves the shape unstyled rather than cut.
    #[test]
    fn each_corner_word_is_a_different_cut() {
        assert_eq!(corner_style("round"), CORNER_ROUND);
        assert_eq!(corner_style("chamfer"), CORNER_CHAMFER);
        assert_eq!(corner_style("square"), CORNER_SQUARE);
        assert_eq!(corner_style("hexagon"), CORNER_SQUARE);
    }
}

#[cfg(test)]
mod sentinel_tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static RADII: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    extern "C" fn rec_ring_fill(_: *mut c_void, _: RectC, _: u32, radius: f32, _: ColorC) {
        RADII.with(|r| r.borrow_mut().push(radius));
    }

    extern "C" fn rec_ring(_: *mut c_void, _: RectC, _: u32, radius: f32, _: f32, _: ColorC) {
        RADII.with(|r| r.borrow_mut().push(radius));
    }

    thread_local! {
        static QUADS: RefCell<Vec<[f32; 8]>> = const { RefCell::new(Vec::new()) };
    }

    extern "C" fn rec_quad(_: *mut c_void, pts: *const f32, _: ColorC) {
        let mut q = [0.0f32; 8];
        // The ABI's quad is eight floats at `pts`; the entry is only
        // ever reached from this file's own `chamfer_fill`.
        q.copy_from_slice(unsafe { std::slice::from_raw_parts(pts, 8) });
        QUADS.with(|v| v.borrow_mut().push(q));
    }

    /// A rung that draws both halves of the frame — a fill to carry the
    /// `ring_fill` radius and a stroke to carry the `ring` one — and no
    /// glow, so the mask sprite stays out of the count.
    const LIT: StateStyleC = StateStyleC {
        fill: ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        edge: ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        text: NO_COLOR,
        glyph: NO_COLOR,
        edge_width: 1.0,
        glow_radius: 0.0,
        glow_alpha: 0.0,
        elevation: 0.0,
    };

    /// Every radius `frame` hands the ring pair for `cell`, taken through
    /// the real function: a host table whose two ring entries write the
    /// radius down and whose `api_size` is the live one, so `has_ring`
    /// answers as it does in the running program.
    fn draw(corner: Corner, cell: RectC) -> (Vec<f32>, Vec<[f32; 8]>) {
        let api = HostApi {
            ring_fill: rec_ring_fill,
            ring: rec_ring,
            quad: rec_quad,
            ..*nacelle::plugin::host_api()
        };
        RADII.with(|r| r.borrow_mut().clear());
        QUADS.with(|v| v.borrow_mut().clear());
        // A null drawing context: every entry reached here is ours and
        // none of them looks at it.
        frame(&api, std::ptr::null_mut(), cell, corner, &LIT, 0.0);
        (RADII.with(|r| r.borrow().clone()), QUADS.with(|v| v.borrow().clone()))
    }

    fn radii(corner: Corner, cell: RectC) -> Vec<f32> {
        draw(corner, cell).0
    }

    /// `@corner.pill` on a tile's radius token is a CAPSULE, and the
    /// capsule is half the shorter side of the box it is a word about.
    /// The clamp this line used to carry (`radius.max(0.0)`) made it a
    /// square, which is the shape the master wrote `pill` to avoid —
    /// and a return to that clamp fails here rather than passing.
    #[test]
    fn a_pill_radius_reaches_the_ring_pair_as_half_the_short_side() {
        let pill = nacelle::theme::expr::sentinel("pill").expect("§5.0 declares pill");
        let cell = RectC { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let got = radii(Corner { style: CORNER_ROUND, radius: pill }, cell);
        assert_eq!(got, vec![20.0, 20.0], "the fill's radius and the ring's");
        // The short side is whichever it is: a tall tile capsules across
        // its width, and a clamp that happened to equal h/2 cannot pass
        // both of these.
        let tall = RectC { x: 0.0, y: 0.0, w: 40.0, h: 200.0 };
        assert_eq!(radii(Corner { style: CORNER_ROUND, radius: pill }, tall), vec![20.0, 20.0]);
    }

    /// The chamfer path never reaches the ring pair — it is built here,
    /// out of quads — so it is the one cut no door below this file could
    /// have rescued. A capsule chamfer is the widest bevel the box holds.
    #[test]
    fn a_pill_radius_cuts_the_chamfer_by_the_same_half_side() {
        let pill = nacelle::theme::expr::sentinel("pill").expect("§5.0 declares pill");
        let cell = RectC { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let (radii, quads) = draw(Corner { style: CORNER_CHAMFER, radius: pill }, cell);
        assert!(radii.is_empty(), "a chamfer is quads, not a ring");
        // `chamfer_fill`'s top trapezoid: its first vertex sits one cut
        // in from the left edge. A cut of zero — which is what the
        // retired clamp made of the sentinel — would put it ON the edge,
        // and the else-arm would draw a plain rect with no quad at all.
        let top = quads.first().expect("the sentinel was read as no cut at all");
        assert_eq!(top[0], cell.x + 20.0, "the top-left cut starts where the capsule says");
    }

    /// A length is still a length: the sentinel reading must not move
    /// the radius a master actually states, and the other sentinels —
    /// `auto`, `same_as_parent` — are the ABSENCE of a length and draw
    /// no corner rather than one this file invents.
    #[test]
    fn a_stated_length_is_untouched_and_the_other_sentinels_cut_nothing() {
        let cell = RectC { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let got = radii(Corner { style: CORNER_ROUND, radius: 6.0 }, cell);
        assert_eq!(got, vec![6.0, 6.0]);
        for word in ["auto", "same_as_parent"] {
            let s = nacelle::theme::expr::sentinel(word).expect(word);
            let got = radii(Corner { style: CORNER_ROUND, radius: s }, cell);
            assert_eq!(got, vec![0.0, 0.0], "{word} is not a radius");
        }
    }
}
