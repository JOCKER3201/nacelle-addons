//! FILESYSTEM panel — icon grid like eDEX-UI, tracks the shell's working
//! directory (from /proc/<pid>/cwd); clicking a directory cds the active
//! terminal tab, clicking a file opens it with the associated application.

use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_NONE,
    ACTION_OPEN_DIR, ACTION_OPEN_FILE, MASK_QUAD_ADD,
};
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::Instant;

/// The interface font, as the host numbers them.
const FONT_UI: u32 = 0;

/// A rectangle in the widget's own arithmetic; `RectC` is what crosses.
#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
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
const STATE_IDLE: u32 = 0;
const STATE_HOVER: u32 = 1;
/// `filetile.row_justify` declares `pack | fill`; the baked enum is the
/// word's index in that list.
const ROW_JUSTIFY_FILL: u32 = 1;

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
    sb_mode: u32,      // scrollbar.mode — declares `overlay | inset | none`; overlay = 0
    sb_w: u32,         // scrollbar.w
    sb_margin: u32,    // scrollbar.margin
    sb_thumb_min: u32, // scrollbar.thumb_min
    sb_side: u32,      // scrollbar.edge — declares `right | left`; right = 0
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
            sb_mode: token(api, "scrollbar.mode"),
            sb_w: token(api, "scrollbar.w"),
            sb_margin: token(api, "scrollbar.margin"),
            sb_thumb_min: token(api, "scrollbar.thumb_min"),
            sb_side: token(api, "scrollbar.edge"),
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
    thumb: StateStyleC,
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
    sb_mode: u32,
    sb_w: f32,
    sb_margin: f32,
    sb_thumb_min: f32,
    sb_side: u32,
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
            sb_mode: 0,
            sb_w: 0.0,
            sb_margin: 0.0,
            sb_thumb_min: 0.0,
            sb_side: 0,
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
            sb_mode: (api.theme_enum)(ctx, t.sb_mode),
            sb_w: px(t.sb_w),
            sb_margin: px(t.sb_margin),
            sb_thumb_min: px(t.sb_thumb_min),
            sb_side: (api.theme_enum)(ctx, t.sb_side),
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

pub struct Filesystem {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    pub scroll: f32,
    /// The path the last click produced, kept alive until the next one.
    last_path: Vec<u8>,
    /// Tile rectangles from the last frame.
    hits: Vec<(Rect, usize)>,
    last_refresh: Instant,
    error: Option<String>,
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<ThemeIds>,
    /// `filetile.wheel_px`, cached at draw because a wheel event arrives
    /// with no drawing context to ask.
    wheel_px: f32,
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
            scroll: 0.0,
            last_path: Vec::new(),
            hits: Vec::new(),
            last_refresh: Instant::now() - std::time::Duration::from_secs(60),
            error: None,
            theme: None,
            wheel_px: 0.0,
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
                self.scroll = 0.0;
                self.refresh();
            }
        }
        if self.last_refresh.elapsed().as_secs() >= 2 {
            self.refresh();
        }
    }

    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).max(0.0);
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
            self.scroll = 0.0;
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
        let look = self.look(api, ctx);
        self.wheel_px = look.wheel_px;

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

        // Scrolling snaps to whole rows — only fully fitting rows are
        // drawn, nothing sticks out of the panel.
        let row_h = tile + gap;
        let total_rows = self.entries.len().div_ceil(cols);
        let nvis = if row_h > 0.0 {
            (((area.h + gap) / row_h).floor() as usize).max(1)
        } else {
            1
        };
        let max_off = total_rows.saturating_sub(nvis);
        self.scroll = self.scroll.clamp(0.0, (max_off as f32 * row_h).max(0.0));
        let row_off = if row_h > 0.0 {
            ((self.scroll / row_h).round() as usize).min(max_off)
        } else {
            0
        };
        // filetile.row_justify = fill stretches the pitch so the last
        // visible row ends exactly at the panel's bottom edge (level with
        // the keyboard); pack sits every row on filetile.gap.
        let step = if look.row_justify == ROW_JUSTIFY_FILL && total_rows > nvis && nvis > 1
        {
            (area.h - tile) / (nvis as f32 - 1.0)
        } else {
            row_h
        };

        // Where the pointer is, once per frame; NaN matches no tile.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);

        for (i, entry) in self.entries.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            if row < row_off || row >= row_off + nvis {
                continue;
            }
            let x = area.x + col as f32 * (tile + gap);
            let y = area.y + (row - row_off) as f32 * step;
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

            self.hits.push((trect, i));
        }

        // The scrollbar, drawn at last (u2 §2.10): the user can see there
        // is more, and where. Overlay only — the master's own word, and
        // the one an enum index can decode across the ABI; any other word
        // (inset, none) draws nothing until the ABI can tell them apart.
        // The thumb is the scrollbar.thumb class's idle rung; row
        // snapping stays exactly as it was.
        if total_rows > nvis && look.sb_mode == 0 && look.sb_w > 0.0 {
            let bw = look.sb_w;
            let bx = if look.sb_side == 0 {
                area.right() - look.sb_margin - bw
            } else {
                area.x + look.sb_margin
            };
            let frac = (nvis as f32 / total_rows as f32).clamp(0.0, 1.0);
            let th = (area.h * frac).max(look.sb_thumb_min).min(area.h);
            let ty = area.y
                + (area.h - th) * (row_off as f32 / max_off.max(1) as f32).clamp(0.0, 1.0);
            let thumb = RectC { x: bx, y: ty, w: bw, h: th };
            if look.thumb.fill.a > 0.0 {
                (api.rect)(ctx, thumb, look.thumb.fill);
            }
            if look.thumb.edge_width > 0.0 && look.thumb.edge.a > 0.0 {
                (api.rect_outline)(ctx, thumb, look.thumb.edge_width, look.thumb.edge);
            }
        }
    }
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
        // filetile.wheel_px, as the last draw cached it — a wheel event
        // arrives with no drawing context to ask the theme through.
        let px = this.wheel_px;
        this.wheel(dy * px);
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
