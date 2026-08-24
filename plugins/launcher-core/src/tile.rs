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

use nacelle::plugin_shapes;
use nacelle::runtime::{ColorC, HostApi, RectC, StateStyleC, CORNER_ROUND};
#[cfg(test)]
use nacelle::runtime::{CORNER_CHAMFER, CORNER_SQUARE};
use nacelle::ui::Case;
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
    /// [`nacelle::runtime::CORNER_SQUARE`], [`CORNER_ROUND`] or
    /// [`nacelle::runtime::CORNER_CHAMFER`].
    pub style: u32,
    pub radius: f32,
}

/// The cut a `*_corner_style` word names — [`nacelle::corner::code_of`],
/// the shared reader every plugin's `*_corner_style` now goes through
/// rather than a match of its own. A word this build has never heard of
/// leaves the shape square, which is what an unstyled rectangle already
/// is — never a cut of this file's choosing.
pub fn corner_style(word: &str) -> u32 {
    nacelle::corner::code_of(word)
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

// ------------------------------------------------------- the empty state
//
// The line a panel draws INSTEAD of its content. `[emptystate]` is two
// keys — `y_frac`, where in the box it sits, and `role`, which type it is
// set in — and the second is the whole reason this pair lives here rather
// than in each widget: "no applications" said by the grid and the same
// sentence said by the list beside it are ONE kind of element, and the
// master gives that kind one answer.
//
// It was three answers. The grid drew its line in the tile CAPTION's role
// (`type.caption`, 9.6 px at 1080 lines), the categories list drew it in
// its ROW's role (`type.body`, 13.3 px) with a comment arguing that a
// list's empty line is a row, and the search panel and the settings
// window read `emptystate.role` (`type.value`, 17.6 px) — a spread of
// 84 % between two widgets standing side by side on the same board.
// Neither local argument was wrong about its own panel; both were
// answering a question the theme had already answered.

/// Token ids the empty state draws from, resolved by NAME once per epoch.
pub struct EmptyTheme {
    pub epoch: u32,
    /// `emptystate.y_frac` — where in the content box the line sits.
    y_frac: u32,
    // type.<emptystate.role>.* — the role the group above names.
    size: u32,
    min: u32,
    tracking: u32,
    leading: u32,
    case: u32,
    fg: u32,
    /// The slot that role's `face` names, resolved WITH the ids because
    /// a face is a word and reading words is init-time work.
    font: u32,
}

impl EmptyTheme {
    pub fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> EmptyTheme {
        // The binding, followed to the role it names. A master that binds
        // no role leaves every id below MISSING, which is a line of no
        // size: naming a role here would be this file choosing the type.
        let role = enum_word(api, ctx, token(api, "emptystate.role"));
        EmptyTheme {
            epoch,
            y_frac: token(api, "emptystate.y_frac"),
            size: role_id(api, &role, "size"),
            min: role_id(api, &role, "min_px"),
            tracking: role_id(api, &role, "tracking"),
            leading: role_id(api, &role, "leading"),
            case: role_id(api, &role, "case"),
            fg: role_id(api, &role, "fg"),
            font: face_slot(api, ctx, role_id(api, &role, "face")),
        }
    }
}

/// The empty state's values for one frame, read fresh from the ids.
#[derive(Clone, Copy)]
pub struct EmptyLook {
    pub y_frac: f32,
    pub px: f32,
    pub tracking: f32,
    pub leading: f32,
    pub case: Case,
    pub font: u32,
    pub ink: ColorC,
}

impl EmptyLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// No ink, zero lengths — type of no size draws nothing, which is
    /// the same undesigned raw an empty theme gives.
    pub fn raw() -> EmptyLook {
        EmptyLook {
            y_frac: 0.0,
            px: 0.0,
            tracking: 0.0,
            leading: 1.0,
            case: Case::None,
            font: FONT_UI,
            ink: NO_COLOR,
        }
    }

    pub fn read(api: &HostApi, ctx: *mut c_void, t: &EmptyTheme) -> EmptyLook {
        let px = |id| (api.theme_px)(ctx, id);
        EmptyLook {
            y_frac: px(t.y_frac),
            px: px(t.size).max(px(t.min)),
            tracking: px(t.tracking),
            // A leading below one line would stack the lines on top of
            // each other; the role declares 1.0 .. 2.0 and the floor is
            // what a nonsense value degrades to, not a chosen pitch.
            leading: px(t.leading).max(1.0),
            case: Case::from_word(&enum_word(api, ctx, t.case)),
            font: t.font,
            ink: (api.theme_color)(ctx, t.fg),
        }
    }
}

/// The one line a panel with nothing to show draws, centred on `r` at
/// `emptystate.y_frac` — the whole element, so that the two launcher
/// widgets cannot draw it two ways again.
///
/// `ink` overrides the role's own `fg` when a caller has a better answer
/// (a row list tints its line with the row class's text ink so the line
/// sits in the same colour the rows would have); `None` takes the role's.
pub fn empty_line(
    api: &HostApi,
    ctx: *mut c_void,
    look: &EmptyLook,
    r: Rect,
    what: &str,
    ink: Option<ColorC>,
) {
    if look.px <= 0.0 {
        return;
    }
    let sp = look.px * look.tracking;
    let text = recase(look.case, what);
    // The fraction says where in the box the line sits; the role's own
    // leading is what centres the LINE BOX on it rather than hanging the
    // glyphs below it.
    let y = r.y + r.h * look.y_frac - look.px * look.leading / 2.0;
    self::text(
        api,
        ctx,
        look.font,
        look.px,
        r.cx(),
        y,
        &text,
        ink.unwrap_or(look.ink),
        sp,
        1,
    );
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
    pub caption_case: Case,
    pub caption_font: u32,
    /// `type.ellipsis` — what a name this grid had to cut ends on.
    ///
    /// A STRING and not an id, because a text token is not baked into
    /// the table the other kinds are read from: the host answers it by
    /// scanning every text key the theme declares, under the engine's
    /// global lock, which is why its ABI entry documents itself as
    /// init-time. It is read HERE, once per epoch, beside the case word
    /// and every other resolved value, and handed to
    /// [`fit_name`] — never read on the draw path, where it cost two
    /// crossings of the ABI and two turns of that lock per trimmed name
    /// per frame.
    ///
    /// EMPTY is the honest answer twice over: a theme that declares no
    /// key, and a host too old to be asked. Both mean a cut that goes
    /// unmarked, and a widget must not be able to tell them apart.
    pub ellipsis: String,
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
            caption_case: Case::None,
            caption_font: FONT_UI,
            ellipsis: String::new(),
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
            caption_case: Case::from_word(&enum_word(api, ctx, t.caption_case)),
            caption_font: t.caption_font,
            ellipsis: api.theme_text_of(ctx, "type.ellipsis"),
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

// ------------------------------------------------------------------- icons
//
// K8: a real SVG, rasterized on the HOST side into a coverage mask and
// packed into the shared glyph atlas — never a per-icon distance field,
// and never a texel this crate can address directly. See
// `nacelle::icon`'s own doc comment (in libnacelle) for the rasterizer
// and `nacelle::runtime::HostApi::icon_register`/`icon_quad` for the
// boundary these two helpers are the plugin-side shorthand of.

/// The launcher's one bundled fallback icon — four rounded squares, the
/// generic "apps" glyph, embedded at compile time exactly like this
/// crate's own `.meta` files (`include_str!`, `lib.rs`). `appgrid`'s own
/// module doc names the real gap this stands in for: "`Icon=` names a
/// name in a registry this program does not keep" — resolving THAT is a
/// future call to [`HostApi::icon_register`] with an SVG read for the
/// application in hand, at the same call site this constant feeds today.
const APP_GENERIC_ICON_SVG: &[u8] = include_bytes!("../assets/app-generic.svg");

/// Registers [`APP_GENERIC_ICON_SVG`] on `ctx`'s own icon atlas —
/// [`HostApi::icon_register`] interns by NAME, so every tile drawn
/// through the SAME `ctx` this frame reaches the one id after the
/// first — and answers it, or `None` on a host too old to carry the
/// icon path at all ([`HostApi::has_icon`]).
///
/// Called once per tile rather than cached in a `static`: an icon id is
/// meaningful only against the [`nacelle::font::FontSystem`] instance
/// that issued it, and a desktop with more than one output holds one
/// instance PER output (`nacelle::font`'s own doc comment, on the atlas
/// baked per unit size) — a single process-wide cache would hand a
/// second monitor's draw call an id that happens to be a DIFFERENT icon
/// on ITS atlas. The cost of asking again is one hashmap lookup by name
/// inside `icon_register`, not a re-parse of the SVG; a per-`ctx` cache
/// keyed by the pointer's identity would remove even that, and is named
/// here as the follow-up rather than built now.
fn app_generic_icon(api: &HostApi, ctx: *mut c_void) -> Option<u32> {
    if !api.has_icon() {
        return None;
    }
    let name = "nacelle.launcher.app-generic";
    let id = (api.icon_register)(
        ctx,
        name.as_ptr(),
        name.len() as u32,
        APP_GENERIC_ICON_SVG.as_ptr(),
        APP_GENERIC_ICON_SVG.len() as u32,
    );
    (id != u32::MAX).then_some(id)
}

/// Draws icon `id` centred on (`cx`, `cy`), `px` texels on a side,
/// tinted by `c` — [`HostApi::icon_quad`] across the boundary, the
/// icon-side twin of [`chamfer_glow`]'s `mask_quad` call a few lines
/// above it in this file.
fn icon_quad(api: &HostApi, ctx: *mut c_void, id: u32, px: f32, cx: f32, cy: f32, c: ColorC) {
    let px = px.round().max(1.0);
    let half = px / 2.0;
    let pts: [f32; 8] = [
        cx - half, cy - half,
        cx + half, cy - half,
        cx + half, cy + half,
        cx - half, cy + half,
    ];
    (api.icon_quad)(ctx, id, px, pts.as_ptr(), c);
}

/// The font slot a type role's `face` token names.
///
/// A face is a CLOSED word set of eight — the master declares eight
/// `[face.*]` blocks and numbers them itself — and it is read as a WORD
/// rather than as an index, because an index is meaningful only against
/// that numbering and a theme is free to reorder its own file.
///
/// The word→slot rule is the TOOLKIT's: `nacelle::font::face_slot`, the
/// one place that holds the master's list. This used to be a copy of it
/// that could only answer "mono or not", which meant a widget drawing
/// `ui_medium` and the toolkit drawing `ui_medium` reached the same two
/// slots by two different rules — and neither reached the third.
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
    nacelle::font::face_slot(word) as u32
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

/// Trims text (with a trailing marker) so it fits the given width.
///
/// The marker is `type.ellipsis`. It was `"\u{2026}"` written into this
/// function, in a widget whose whole rule is that nothing here decides
/// anything — and the master had declared the key and named this very
/// call site in its comment ("a console theme may prefer `...` or `>`")
/// the whole time. A host too old to answer text tokens, or a theme that
/// states none, passes the EMPTY string and the cut goes unmarked: the
/// cut still happens, which is what a key nobody wrote honestly means.
///
/// `cut` is a PARAMETER and not a read, and that is the whole point of
/// it. [`nacelle::runtime::HostApi::theme_text`] is documented init-time
/// — "call at widget init, cache, invalidate on `theme_epoch`" — because
/// the host answers it by interning the name, reading the id back to a
/// name and scanning every text key the theme declares, and it takes the
/// engine's global lock TWICE on the way. Read here, that bill fell once
/// per trimmed name per frame: a directory of two hundred long names at
/// 60 Hz took the theme engine's lock twenty-four thousand times a
/// second. It belongs in the caller's per-epoch [`TileLook`], where the
/// case word and every other resolved value already sits, and passing it
/// in is how a caller is made to put it there.
#[allow(clippy::too_many_arguments)]
pub fn fit_name(
    api: &HostApi,
    ctx: *mut c_void,
    font: u32,
    px: f32,
    text: &str,
    max_w: f32,
    spacing: f32,
    cut: &str,
) -> String {
    if measure(api, ctx, font, px, text, spacing) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 1 {
        let cand: String = chars[..n].iter().collect::<String>() + cut;
        if measure(api, ctx, font, px, &cand, spacing) <= max_w {
            return cand;
        }
        n -= 1;
    }
    cut.to_string()
}

/// A type role's case transform, applied here because the text entry
/// draws bytes as given.
///
/// The transform itself is the TOOLKIT's — `nacelle::ui::recase`, the one
/// applier the panel band, the window title and the unit suffix go
/// through — so a word the master's list does not hold answers the same
/// way on both sides of the boundary. What is left here is the crossing:
/// the word arrives through `theme_enum_word`, not as an INDEX into the
/// enum, because an index only names a word against the schema it was
/// interned in and this side has no schema at all.
pub fn recase(case: Case, s: &str) -> String {
    nacelle::ui::recase(case, s).into_owned()
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
    // The shape itself, the degrade included, is [`nacelle::plugin_shapes`]
    // now — ONE octagon for every plugin that needs one, in place of the
    // copy this file carried until K7 (`chamfer_fill`/`chamfer_frame`/
    // `chamfer_glow`/`octagon`, all gone from here).
    plugin_shapes::ring_fill(api, ctx, cell, corner.style, cut, rung.fill);
    if rung.edge_width > 0.0 && rung.edge.a > 0.0 {
        plugin_shapes::ring(api, ctx, cell, corner.style, cut, rung.edge_width, rung.edge);
        // Right after the stroke, the ring's glow — this rung's own
        // `glow_radius` and `glow_alpha` from the ladder, tinted with
        // the edge's resolved colour and scaled by the one global knob.
        // Every shipped idle rung is dark; hover and press are where the
        // ladder lights up.
        let alpha = (rung.glow_alpha * glow_scale).clamp(0.0, 1.0);
        if rung.glow_radius > 0.0 && alpha > 0.0 {
            let c = ColorC { a: alpha, ..rung.edge };
            plugin_shapes::ring_glow(api, ctx, cell, corner.style, cut, rung.glow_radius, c);
        }
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
    /// How far down the scroll can go, in the pixels the caller keeps
    /// its offset in — the bottom [`layout`] clamps to, remembered
    /// rather than recomputed so a dragged thumb and the clamp cannot
    /// disagree.
    pub max_px: f32,
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
    let max_px = (max_off as f32 * row_h).max(0.0);
    *scroll = scroll.clamp(0.0, max_px);
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
    Layout { tile, gap, cols, total_rows, nvis, max_off, row_off, step, max_px }
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
    match app_generic_icon(api, ctx) {
        // K8's first real call site: a bundled SVG, rasterized to a
        // coverage mask and drawn tinted by the SAME colour the
        // initial-letter mark used — the icon nobody could draw is now
        // drawn, even though every tile draws the one GENERIC glyph
        // today rather than the application's own art. That is the
        // stated gap `appgrid`'s own doc comment names ("`Icon=` names
        // a name in a registry this program does not keep") and is a
        // change of SOURCE for a follow-up (per-app SVGs, resolved by
        // name), not of plumbing: this call site is the plumbing.
        Some(id) => icon_quad(api, ctx, id, gpx, icon.cx(), icon.cy(), rung.glyph),
        // An old host (no `icon_register`/`icon_quad` — `api.has_icon()`
        // false) draws exactly what it always drew.
        None if !mark.is_empty() => {
            draw_text(api, ctx, gpx, icon.cx(), icon.cy() - gpx / 2.0, mark, rung.glyph, 0.0);
        }
        None => {}
    }

    let px = look.caption_px;
    let sp = px * look.caption_tracking;
    let name = recase(look.caption_case, label);
    let name = fit_name(api, ctx, look.caption_font, px, &name, t.w, sp, &look.ellipsis);
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
/// how many are on screen, which is first, and — in the pixels the
/// caller keeps its own offset in — where that first one stands and how
/// far it may go. A grid of tiles and a list of rows differ in what a
/// row IS and in nothing else, which is why the bar below takes this
/// and not either widget's own layout.
///
/// The position is a PIXEL figure and not the index beside it because
/// the hand that drags the thumb speaks pixels: a thumb drawn from an
/// index and grabbed in pixels agrees only where every unit is the same
/// height. The alphabetical index's are not — a heading is shorter than
/// a row of tiles — so said in indices the two disagree by however much
/// the bands differ, and the thumb walks away from the hand holding it.
/// Said in one unit they are each other's inverse by construction.
#[derive(Clone, Copy)]
pub struct Scroll {
    pub total: usize,
    pub nvis: usize,
    /// The first row or band on screen.
    pub off: usize,
    /// Where that first one's top is, measured from the column's.
    pub px: f32,
    /// The furthest down that top may go.
    pub max_px: f32,
}

impl Layout {
    /// This grid, as the scrollbar reads it.
    pub fn scroll(&self) -> Scroll {
        Scroll {
            total: self.total_rows,
            nvis: self.nvis,
            off: self.row_off,
            // Every row of a flat grid is the same height, so the top of
            // the first one on screen is its index times the pitch the
            // bottom was measured with. `filetile.row_justify` stretches
            // what is DRAWN, never what is scrolled through, which is
            // why this is `tile + gap` and not [`Layout::step`].
            px: self.row_off as f32 * (self.tile + self.gap),
            max_px: self.max_px,
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
    let Some(g) = bar_geom(look, area, s) else { return };
    let thumb = g.thumb.c();
    if look.thumb.fill.a > 0.0 {
        (api.rect)(ctx, thumb, look.thumb.fill);
    }
    if look.thumb.edge_width > 0.0 && look.thumb.edge.a > 0.0 {
        (api.rect_outline)(ctx, thumb, look.thumb.edge_width, look.thumb.edge);
    }
}

/// The bar as a pair of rectangles, or none when the theme asks for no
/// bar or there is nothing to scroll.
///
/// Split out of [`scrollbar`] because a bar the hand can take hold of
/// needs the SAME two rectangles the eye was shown — a second copy of
/// this arithmetic beside the hit test would be a thumb that is drawn
/// in one place and grabbed in another the moment either is touched.
pub fn bar_geom(look: &TileLook, area: Rect, s: Scroll) -> Option<BarGeom> {
    if !(s.total > s.nvis && look.sb_mode != BarMode::None && look.sb_w > 0.0) {
        return None;
    }
    let bw = look.sb_w;
    let bx = if look.sb_side == 0 {
        area.right() - look.sb_margin - bw
    } else {
        area.x + look.sb_margin
    };
    let frac = (s.nvis as f32 / s.total as f32).clamp(0.0, 1.0);
    let th = (area.h * frac).max(look.sb_thumb_min).min(area.h);
    // How far down its travel the thumb sits: the offset over the
    // bottom, both in the caller's own pixels. A column whose units are
    // all one height reads the same either way; one whose units are not
    // reads right only this way. A bottom of nothing is a column that
    // cannot move, and a thumb that cannot move sits at the top.
    let pos = if s.max_px > 0.0 { (s.px / s.max_px).clamp(0.0, 1.0) } else { 0.0 };
    let ty = area.y + (area.h - th) * pos;
    Some(BarGeom {
        track: Rect::new(bx, area.y, bw, area.h),
        thumb: Rect::new(bx, ty, bw, th),
        max_px: s.max_px,
    })
}

/// The bar the last frame drew: the full length the thumb travels in,
/// the thumb AS DRAWN — `scrollbar.thumb_min` may have stretched it,
/// and a grab must be tested against what the eye saw — and the bottom
/// the offset behind it may reach.
///
/// The bottom travels WITH the rectangles rather than beside them
/// because a drag converts between the two, and a widget keeping them in
/// two fields is a widget that can update one and forget the other.
#[derive(Clone, Copy)]
pub struct BarGeom {
    pub track: Rect,
    pub thumb: Rect,
    pub max_px: f32,
}

/// A thumb under the hand, between two frames.
///
/// The toolkit already owns this gesture in `nacelle::view::scroll`, and
/// this is deliberately the same arithmetic and not a second opinion:
/// the offset follows the hand ABSOLUTELY, which is the only behaviour
/// that survives a dropped frame. What the toolkit's version cannot be
/// given here is the offset itself — these two widgets scroll a column
/// of ROWS they clamp themselves, in their own layout pass, so the
/// number the drag produces has to be handed back rather than kept.
#[derive(Clone, Copy, Default)]
pub struct ThumbGrab {
    /// How far down the thumb the hand took hold, and how tall the thumb
    /// was when it did. Kept from the press so the thumb does not jump
    /// under the finger on the first motion.
    held: Option<(f32, f32)>,
}

impl ThumbGrab {
    /// The pointer took hold of the thumb. `false` leaves the press for
    /// whatever else the widget does with it.
    pub fn press(&mut self, y: f32, bar: &BarGeom) -> bool {
        if bar.thumb.h <= 0.0 || y < bar.thumb.y || y >= bar.thumb.y + bar.thumb.h {
            return false;
        }
        self.held = Some((y - bar.thumb.y, bar.thumb.h));
        true
    }

    /// Where the hand has put the offset, in the pixels the widget
    /// scrolls in, or none while nothing is held.
    ///
    /// This is [`bar_geom`]'s own arithmetic run backwards: it puts the
    /// thumb's top at `travel` times the offset over the bottom, and
    /// this reads the offset back off the thumb's top. Grabbed and not
    /// moved, the hand therefore returns the offset it started from.
    pub fn drag_to(&self, y: f32, bar: &BarGeom) -> Option<f32> {
        let (inside, thumb_h) = self.held?;
        let travel = (bar.track.h - thumb_h).max(0.0);
        if travel <= 0.0 {
            return Some(0.0);
        }
        Some(((y - inside - bar.track.y) / travel).clamp(0.0, 1.0) * bar.max_px.max(0.0))
    }

    /// The pointer let go.
    pub fn release(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod token_tests {
    use super::*;

    /// The case transform is the TOOLKIT's, and a word the master's list
    /// does not hold transforms nothing.
    ///
    /// This widget used to hold its own copy, keyed on the ENUM INDEX:
    /// `1 | 3 => to_uppercase()`. Two things were wrong with it — an
    /// index means nothing on this side of the boundary, where there is
    /// no schema to number against, and a copy of a rule is a rule that
    /// stops matching the original. Both end here.
    #[test]
    fn the_case_a_tile_sets_its_caption_in_is_the_toolkits() {
        assert_eq!(recase(Case::from_word("upper"), "Files"), "FILES");
        assert_eq!(recase(Case::from_word("lower"), "Files"), "files");
        assert_eq!(recase(Case::from_word("none"), "Files"), "Files");
        // Smallcaps is drawn as capitals until the host's font layer can
        // set true small caps — the toolkit's approximation, not a
        // second one taken here.
        assert_eq!(recase(Case::from_word("smallcaps"), "Files"), "FILES");
        // A theme with a typo gets NO transform and a line on stderr,
        // where the index-keyed copy would have silently answered the
        // word at that position — or, on the host side, capitals.
        assert_eq!(recase(Case::from_word("uper"), "Files"), "Files");
        // And a host too old to answer words at all hands back the empty
        // string, which is a missing token and not a typo.
        assert_eq!(recase(Case::from_word(""), "Files"), "Files");
    }

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
        // The ABI's quad is eight floats at `pts`; on a host with the
        // ring pair this entry is never reached from `frame` at all —
        // it is `nacelle::plugin_shapes::chamfer_fill`'s, the fallback
        // for a host old enough to lack it.
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

    /// The chamfer path now reaches the SAME ring pair the round path
    /// does: [`nacelle::plugin_shapes::ring_fill`] sends
    /// [`CORNER_CHAMFER`] through [`HostApi::ring_fill`] exactly as it
    /// sends [`CORNER_ROUND`], on any host that carries the pair —
    /// [`HostApi::ring_fill`]'s own fast path draws the octagon on the
    /// host's side (`draw.rs`'s `chamfer_fill_verts`), which is what
    /// this file's `chamfer_fill` copy was rebuilding on this one before
    /// K7. A capsule chamfer is still the widest bevel the box holds,
    /// which is why the radius, not the shape, is what this pins.
    #[test]
    fn a_pill_radius_reaches_the_ring_pair_through_the_chamfer_style_too() {
        let pill = nacelle::theme::expr::sentinel("pill").expect("§5.0 declares pill");
        let cell = RectC { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let (radii, quads) = draw(Corner { style: CORNER_CHAMFER, radius: pill }, cell);
        assert_eq!(radii, vec![20.0, 20.0], "the fill's radius and the ring's");
        assert!(quads.is_empty(), "the ring pair drew it; no quad of this file's own was needed");
    }

    /// A host old enough to lack the ring pair still bevels a chamfer by
    /// hand — the one rung [`nacelle::plugin_shapes::chamfer_fill`]
    /// exists for, reached through `frame` exactly as it always was,
    /// just no longer out of a copy this file kept for itself.
    #[test]
    fn an_old_host_still_bevels_a_chamfer_by_hand() {
        let cell = RectC { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        let api = HostApi {
            api_size: nacelle::runtime::HOST_API_HAS_CLIP as u32,
            quad: rec_quad,
            ..*nacelle::plugin::host_api()
        };
        assert!(!api.has_ring(), "the fixture must predate the ring pair");
        QUADS.with(|v| v.borrow_mut().clear());
        frame(
            &api,
            std::ptr::null_mut(),
            cell,
            Corner { style: CORNER_CHAMFER, radius: 8.0 },
            &LIT,
            0.0,
        );
        let quads = QUADS.with(|v| v.borrow().clone());
        // The hand-rolled octagon's top trapezoid: its first vertex sits
        // one cut in from the left edge.
        let top = quads.first().expect("an old host must still bevel a chamfer");
        assert_eq!(top[0], cell.x + 8.0, "the top-left cut starts where the length says");
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

#[cfg(test)]
mod binding_tests {
    use super::*;

    /// A binding, followed to its role and to that role's family — the
    /// chain every `*Theme::resolve` above walks, checked against the
    /// master rather than against a name written into the code.
    fn family_of(binding: &str) -> String {
        nacelle::theme::load();
        let id = nacelle::theme::id(binding)
            .unwrap_or_else(|| panic!("the master declares no {binding}"));
        let role = nacelle::theme::enum_word_of(id)
            .unwrap_or_else(|| panic!("{binding} names no word"));
        assert!(!role.is_empty(), "{binding} names no role");
        for suffix in ["size", "min_px", "tracking", "leading", "case", "fg", "face"] {
            let name = role_token(&role, suffix).expect("a bound role names its family");
            assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
        }
        let t = nacelle::theme::resolved();
        let px = t.px(nacelle::theme::id(&role_token(&role, "size").unwrap()).unwrap());
        assert!(px > 0.0, "{binding} lands on a role of no size");
        role
    }

    /// The line a panel draws instead of its content is ONE kind of
    /// element, so it has one binding — and that binding is not the tile
    /// caption's and not a list row's.
    ///
    /// It was both, and neither: the launcher grid drew the sentence in
    /// `tile.caption_role`, the categories list beside it drew the same
    /// sentence in `list.label_role`, and `emptystate.role` — the key the
    /// master declares for exactly this — was read only by the search
    /// panel and the settings window. Three answers to one question, a
    /// spread of 84 % between the two widgets that share a board.
    #[test]
    fn an_empty_panel_and_a_tile_caption_are_two_different_bindings() {
        let empty = family_of("emptystate.role");
        let caption = family_of("tile.caption_role");
        let row = family_of("list.label_role");
        assert_ne!(
            empty, caption,
            "the master binds an empty state and a tile caption to one role, so \
             this test cannot tell a grid that reads `emptystate.role` from one \
             that reads its caption's"
        );
        assert_ne!(empty, row, "the same, for the row role the categories list drew it in");
    }

    /// A launcher's alphabetical break is a HEADING, and the master says
    /// which role a heading is set in exactly once, in `table.head_role`.
    /// `sections.rs` spelled `type.label.section.*` out instead, so two
    /// of the trio it takes from `[table]` moved with the theme and the
    /// type did not.
    #[test]
    fn an_alphabetical_break_takes_the_heading_binding_whole() {
        family_of("table.head_role");
    }
}

#[cfg(test)]
mod bar_tests {
    use super::*;

    /// A bar the theme really asks for: six wide, no margin, on the
    /// right, with a floor of four on the thumb.
    fn look() -> TileLook {
        TileLook {
            sb_mode: BarMode::Overlay,
            sb_w: 6.0,
            sb_margin: 0.0,
            sb_thumb_min: 4.0,
            sb_side: 0,
            ..TileLook::raw()
        }
    }

    /// That bar over a grid of tiles: twenty to the side, two across,
    /// gapless, so a row is twenty tall and the arithmetic below is the
    /// kind a reader does in their head. No floor under the thumb —
    /// where a thumb sits and how short it may get are two questions.
    fn grid_look() -> TileLook {
        TileLook { cell_pref: 20.0, cell_min: 1.0, cols: 2.0, sb_thumb_min: 0.0, ..look() }
    }

    const AREA: Rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

    /// A column whose units are all one height — the shape a flat grid
    /// of tiles and a list of rows both have — said once here, so the
    /// assertions below are about the BAR and not about either.
    fn even(total: usize, nvis: usize, off: usize, unit: f32) -> Scroll {
        Scroll {
            total,
            nvis,
            off,
            px: off as f32 * unit,
            max_px: total.saturating_sub(nvis) as f32 * unit,
        }
    }

    /// The two rectangles the hand is offered: a track down the whole
    /// content box, and a thumb inside it whose length is the visible
    /// share and whose position is how far down the list has come. The
    /// track spans the box rather than the thumb, because "beside the
    /// thumb" has to be a place a press can land.
    #[test]
    fn the_track_spans_the_box_and_the_thumb_says_where_the_eye_is() {
        let top = bar_geom(&look(), AREA, even(20, 5, 0, 20.0))
            .expect("twenty rows through five is a bar");
        assert_eq!((top.track.x, top.track.y, top.track.w, top.track.h), (94.0, 0.0, 6.0, 100.0));
        // A quarter on show is a quarter of the track long, at the top.
        assert_eq!((top.thumb.x, top.thumb.y, top.thumb.w, top.thumb.h), (94.0, 0.0, 6.0, 25.0));
        // The bar carries the bottom the offset behind it may reach, so
        // a hand taking hold of it needs nothing else.
        assert_eq!(top.max_px, 300.0);
        // At the bottom of the list the thumb ends where the track does,
        // which is what makes "the last row" visible as a position.
        let end = bar_geom(&look(), AREA, even(20, 5, 15, 20.0)).unwrap();
        assert_eq!(end.thumb.y + end.thumb.h, AREA.y + AREA.h);
        // And the thumb never falls below the floor the theme sets, so a
        // very long list still leaves something to take hold of.
        let long = bar_geom(&look(), AREA, even(10_000, 5, 0, 20.0)).unwrap();
        assert_eq!(long.thumb.h, 4.0);
    }

    /// No bar, no gesture: a list that fits, a theme that asks for no
    /// bar, and a bar of no width are three ways of having nothing for
    /// the hand to grab — and each answers none rather than a rectangle
    /// of zero size that a press could still land in.
    #[test]
    fn a_list_that_fits_offers_the_hand_nothing() {
        let fits = even(5, 5, 0, 20.0);
        assert!(bar_geom(&look(), AREA, fits).is_none());
        let scrolls = even(20, 5, 0, 20.0);
        assert!(bar_geom(&TileLook { sb_mode: BarMode::None, ..look() }, AREA, scrolls).is_none());
        assert!(bar_geom(&TileLook { sb_w: 0.0, ..look() }, AREA, scrolls).is_none());
    }

    /// The grab is ABSOLUTE: the offset is where the hand is on the
    /// track, not how far the hand has moved. Held ten pixels down a
    /// twenty-five-pixel thumb and carried to the middle of the box, the
    /// list stands at the middle of its travel.
    #[test]
    fn the_offset_follows_the_hand_and_stops_at_both_ends() {
        let bar = bar_geom(&look(), AREA, even(20, 5, 0, 20.0)).unwrap();
        let mut g = ThumbGrab::default();
        // Beside the thumb is not a grab, and nothing is held after it.
        assert!(!g.press(60.0, &bar));
        assert_eq!(g.drag_to(60.0, &bar), None);

        assert!(g.press(10.0, &bar));
        // 75 px of travel; the hand at 47.5 puts the thumb's top at
        // 37.5, which is half the travel and so half the content.
        assert_eq!(g.drag_to(47.5, &bar), Some(150.0));
        // Past either end of the track the list stops at its own end.
        assert_eq!(g.drag_to(-999.0, &bar), Some(0.0));
        assert_eq!(g.drag_to(999.0, &bar), Some(300.0));
        g.release();
        assert_eq!(g.drag_to(47.5, &bar), None);
    }

    /// A thumb as long as its track has nowhere to travel: the answer is
    /// the top of the list, never a division by zero.
    #[test]
    fn a_thumb_with_no_travel_answers_the_top() {
        let bar = BarGeom {
            track: Rect::new(94.0, 0.0, 6.0, 100.0),
            thumb: Rect::new(94.0, 0.0, 6.0, 100.0),
            max_px: 300.0,
        };
        let mut g = ThumbGrab::default();
        assert!(g.press(50.0, &bar));
        assert_eq!(g.drag_to(90.0, &bar), Some(0.0));
    }

    /// The bottom [`layout`] clamped to, said in the pixels the caller
    /// keeps its offset in, and carried through to the bar that divides
    /// by it.
    ///
    /// Nobody below this function can answer it: the caller has the
    /// offset and not the row height, and a caller that worked the
    /// bottom out for itself would be a second clamp with nothing
    /// holding it to this one. The bar is where that shows — a bottom of
    /// nothing there is a thumb that cannot be dragged anywhere and a
    /// page that cannot be pressed down.
    #[test]
    fn a_grid_reports_the_bottom_it_clamped_to_and_where_it_stands() {
        let look = grid_look();
        // Twenty tiles two across is ten rows of twenty; five fit the
        // hundred-tall box, so five are past the bottom and the bottom
        // is a hundred pixels down.
        let mut s = 0.0;
        let l = layout(&look, AREA, 20, &mut s);
        assert_eq!((l.cols, l.total_rows, l.nvis, l.max_off), (2, 10, 5, 5));
        assert_eq!(l.max_px, 100.0);
        assert_eq!((l.scroll().px, l.scroll().max_px), (0.0, 100.0));
        assert_eq!(bar_geom(&look, AREA, l.scroll()).expect("a bar").max_px, 100.0);
        // Scrolled past the end, the offset is pulled back to that same
        // bottom and the bar is told the same figure, not a second one.
        let mut s = 9999.0;
        let l = layout(&look, AREA, 20, &mut s);
        assert_eq!(s, 100.0);
        assert_eq!((l.row_off, l.scroll().px, l.scroll().max_px), (5, 100.0, 100.0));
        // A grid that fits has nowhere to go: no bottom, and no bar to
        // divide by it.
        let mut s = 40.0;
        let l = layout(&look, AREA, 4, &mut s);
        assert_eq!((l.max_off, l.max_px, s), (0, 0.0, 0.0));
        assert!(bar_geom(&look, AREA, l.scroll()).is_none());
    }
}

#[cfg(test)]
mod trim_tests {
    //! `type.ellipsis` is an INIT-TIME token, and this is the guard that
    //! it stays one.
    //!
    //! `HostApi::theme_text`'s own documentation says so — "call at
    //! widget init, cache, invalidate on `HostApi::theme_epoch`" — and
    //! gives the reason: the host answers a text token by scanning every
    //! text key the theme declares, under the theme engine's global
    //! lock. `theme_text_of` crosses the ABI TWICE on the way (once to
    //! intern the name, once to fetch the string) and takes that lock on
    //! each crossing. On a draw path that is a lock taken per trimmed
    //! name per frame; a directory of two hundred cut names at 60 Hz
    //! took it twenty-four thousand times a second.
    //!
    //! So the marker is a PARAMETER of [`fit_name`], read once per epoch
    //! into the caller's `Look`. Nothing below asserts a speed: it
    //! asserts that the entry is not reached at all while text is being
    //! trimmed, which is the property a speed would follow from.

    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// How many times a trim reached the host's text-token entry.
        static TEXT_CALLS: Cell<u32> = const { Cell::new(0) };
        /// ...and its name-interning half, which `theme_text_of` calls
        /// first and which takes the same lock.
        static TOKEN_CALLS: Cell<u32> = const { Cell::new(0) };
    }

    extern "C" fn counting_text(_: *mut c_void, _: u32, _: *mut u8, _: u32) -> u32 {
        TEXT_CALLS.with(|c| c.set(c.get() + 1));
        0
    }

    extern "C" fn counting_token(_: *const u8, _: u32) -> u32 {
        TOKEN_CALLS.with(|c| c.set(c.get() + 1));
        0
    }

    /// Half an em a character: wrong about fonts, right about
    /// monotonicity, which is all a trim asks of a measurement.
    extern "C" fn ruler(
        _: *mut c_void,
        _: u32,
        px: f32,
        text: *const u8,
        len: u32,
        _: f32,
    ) -> f32 {
        // Every string this module measures is ASCII, so bytes are
        // characters and the count needs no decode.
        let n = unsafe { std::slice::from_raw_parts(text, len as usize) }.len();
        n as f32 * px * 0.5
    }

    /// A host whose text entries are counted and whose ruler is the one
    /// above. Everything else is the real table, so nothing here is a
    /// second implementation of the boundary.
    fn counted() -> HostApi {
        TEXT_CALLS.with(|c| c.set(0));
        TOKEN_CALLS.with(|c| c.set(0));
        HostApi {
            measure: ruler,
            theme_text: counting_text,
            theme_token: counting_token,
            ..*nacelle::plugin::host_api()
        }
    }

    #[test]
    fn trimming_a_name_asks_the_host_for_no_text_token() {
        let api = counted();
        // Twenty characters at 10 px are 100 px wide under this ruler,
        // so 40 px is a cut and the marker is in play.
        let cut = fit_name(&api, std::ptr::null_mut(), FONT_UI, 10.0, "a-very-long-app-name", 40.0, 0.0, "\u{2026}");
        assert!(cut.ends_with('\u{2026}'), "the name was not trimmed: {cut}");
        assert_eq!(
            (TOKEN_CALLS.with(|c| c.get()), TEXT_CALLS.with(|c| c.get())),
            (0, 0),
            "a trim reached the host's text-token entries; the marker \
             belongs in the caller's per-epoch Look, not on this path"
        );
    }

    #[test]
    fn a_name_that_fits_is_returned_whole_and_asks_for_nothing_either() {
        let api = counted();
        let name = "short";
        assert_eq!(fit_name(&api, std::ptr::null_mut(), FONT_UI, 10.0, name, 400.0, 0.0, "\u{2026}"), name);
        assert_eq!((TOKEN_CALLS.with(|c| c.get()), TEXT_CALLS.with(|c| c.get())), (0, 0));
    }

    /// A theme that states no marker, and a host too old to be asked,
    /// arrive here as the SAME empty string — the contract
    /// `HostApi::theme_text` states — and a cut under it goes unmarked
    /// rather than ending on a character this file chose.
    #[test]
    fn an_unstated_marker_trims_the_name_and_marks_nothing() {
        let api = counted();
        let cut = fit_name(&api, std::ptr::null_mut(), FONT_UI, 10.0, "a-very-long-app-name", 40.0, 0.0, "");
        assert_eq!(cut, "a-very-l", "an unmarked cut still has to fit");
        assert!(!cut.contains('\u{2026}'), "a marker nobody stated was drawn anyway");
    }

    /// The marker a theme DOES state is the one that ends the cut —
    /// `>` for a console theme, which is the master's own example.
    #[test]
    fn the_stated_marker_is_what_the_cut_ends_on() {
        let api = counted();
        let cut = fit_name(&api, std::ptr::null_mut(), FONT_UI, 10.0, "a-very-long-app-name", 40.0, 0.0, ">");
        assert!(cut.ends_with('>'), "the cut did not end on the stated marker: {cut}");
        assert!(cut.len() * 5 <= 40, "the marked cut does not fit: {cut}");
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static REGISTERED: RefCell<Vec<(String, usize)>> = const { RefCell::new(Vec::new()) };
        static DRAWN: RefCell<Vec<(u32, f32, [f32; 8], ColorC)>> = const { RefCell::new(Vec::new()) };
    }

    extern "C" fn rec_icon_register(
        _: *mut c_void,
        name: *const u8,
        name_len: u32,
        _svg: *const u8,
        svg_len: u32,
    ) -> u32 {
        let name = unsafe { std::slice::from_raw_parts(name, name_len as usize) };
        let name = std::str::from_utf8(name).unwrap().to_string();
        REGISTERED.with(|r| r.borrow_mut().push((name, svg_len as usize)));
        // A fixed, recognizable id — never `u32::MAX`, so a caller
        // reading it back can tell "registered" from "refused".
        7
    }

    extern "C" fn rec_icon_quad(_: *mut c_void, id: u32, px: f32, pts: *const f32, c: ColorC) {
        let mut p = [0.0f32; 8];
        p.copy_from_slice(unsafe { std::slice::from_raw_parts(pts, 8) });
        DRAWN.with(|v| v.borrow_mut().push((id, px, p, c)));
    }

    fn recording_api() -> HostApi {
        HostApi {
            icon_register: rec_icon_register,
            icon_quad: rec_icon_quad,
            ..*nacelle::plugin::host_api()
        }
    }

    /// [`app_generic_icon`] registers the bundled SVG WHOLE — the byte
    /// count crossing the boundary is the file's own length, not a
    /// truncated or empty stand-in — under one stable name, which is
    /// what lets [`nacelle::font::FontSystem::icon_id`]'s interning give
    /// a second tile drawn through the same `ctx` this frame the SAME
    /// id rather than a fresh parse.
    #[test]
    fn app_generic_icon_registers_the_bundled_svg_by_a_stable_name() {
        let api = recording_api();
        REGISTERED.with(|r| r.borrow_mut().clear());
        let id = app_generic_icon(&api, std::ptr::null_mut());
        assert_eq!(id, Some(7));
        assert!(!APP_GENERIC_ICON_SVG.is_empty());
        assert_eq!(
            REGISTERED.with(|r| r.borrow().clone()),
            vec![("nacelle.launcher.app-generic".to_string(), APP_GENERIC_ICON_SVG.len())]
        );
    }

    /// A host from before the icon pair (`has_icon()` false) answers
    /// `None` WITHOUT calling `icon_register` at all — the same
    /// discipline every other `has_*` gate in this file already keeps,
    /// and what lets [`tile_face`]'s caller fall back to the
    /// initial-letter mark instead of reading past the end of an old
    /// table.
    #[test]
    fn app_generic_icon_is_none_on_a_host_before_the_icon_pair() {
        let api = HostApi {
            api_size: nacelle::runtime::HOST_API_HAS_THEME_TEXT as u32,
            ..recording_api()
        };
        REGISTERED.with(|r| r.borrow_mut().clear());
        assert_eq!(app_generic_icon(&api, std::ptr::null_mut()), None);
        assert!(REGISTERED.with(|r| r.borrow().is_empty()));
    }

    /// [`icon_quad`] centres a square box of `px` texels on `(cx, cy)`
    /// and passes the id and colour through untouched — the same
    /// four-corner convention [`HostApi::quad`] already uses everywhere
    /// else in this file.
    #[test]
    fn icon_quad_centres_a_square_box_on_the_given_point() {
        let api = recording_api();
        DRAWN.with(|v| v.borrow_mut().clear());
        let c = ColorC { r: 0.1, g: 0.2, b: 0.3, a: 1.0 };
        icon_quad(&api, std::ptr::null_mut(), 9, 20.0, 100.0, 50.0, c);
        let drawn = DRAWN.with(|v| v.borrow().clone());
        assert_eq!(drawn.len(), 1);
        let (id, px, pts, colour) = drawn[0];
        assert_eq!(id, 9);
        assert_eq!(px, 20.0);
        assert_eq!((colour.r, colour.g, colour.b, colour.a), (c.r, c.g, c.b, c.a));
        // Four corners of a 20x20 box centred on (100, 50).
        assert_eq!(pts, [90.0, 40.0, 110.0, 40.0, 110.0, 60.0, 90.0, 60.0]);
    }

    /// A non-finite or sub-pixel `px` (the theme's own `icon.size_*`
    /// ladder degrading, or a squeezed grid shrinking the box to
    /// nothing) still draws a real, non-degenerate box: the same floor
    /// [`FontSystem::icon`]'s own `px == 0` refusal makes worth having a
    /// caller-side answer for, rather than asking the host to rasterize
    /// a zero-texel mask every such frame.
    #[test]
    fn icon_quad_floors_a_sub_pixel_size_to_one_texel() {
        let api = recording_api();
        DRAWN.with(|v| v.borrow_mut().clear());
        icon_quad(&api, std::ptr::null_mut(), 1, 0.2, 10.0, 10.0, NO_COLOR);
        let drawn = DRAWN.with(|v| v.borrow().clone());
        assert_eq!(drawn[0].1, 1.0, "a sub-pixel request must not round down to zero");
    }
}
