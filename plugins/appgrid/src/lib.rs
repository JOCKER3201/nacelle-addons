//! APPLICATIONS panel — the launcher grid: every installed XDG desktop
//! entry as a tile, alphabetical, clicking one runs it.
//!
//! It is the file browser's sibling by construction. The grid, the row
//! snapping, the wheel and the scrollbar are the same arithmetic over
//! the same `filetile.*` tokens, because a tile is a tile; what differs
//! is where the list comes from (see [`desktop`]) and what a click does
//! with it.
//!
//! THERE ARE NO ICONS, and that is a stated gap rather than an
//! oversight: the project has no icon theme, `Icon=` names a name in
//! one, and inventing artwork here would be a look this file decided
//! instead of the theme. Until the icon registry can answer, a tile
//! wears the initial of its application in the launcher glyph size —
//! neutral, legible, and nobody's design.
//!
//! Every colour, length, duration and word comes from the theme through
//! ABI 5/6 tokens. Nothing here knows what a colour is: a missing token
//! degrades through the raw answers the ABI itself gives (grey ink,
//! zero lengths), never through a number that used to be the design.

pub mod desktop;

use desktop::AppEntry;
use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_NONE,
    MASK_QUAD_ADD,
};
use std::ffi::c_void;
use std::time::Instant;

/// The interface font, as the host numbers them.
const FONT_UI: u32 = 0;

/// How often the menu is looked at again. The scan itself only runs
/// when the directories' modification times have MOVED, so this is the
/// rate of a handful of `stat` calls, not of a walk.
const RESCAN_SECS: u64 = 5;

// The interaction states, as indices into the matrix's declaration
// order (idle, hover, press, selected, selected_hover, dragging,
// disabled). A tile is a container: every one rests on its class's idle
// rung, the pointed-at one on hover, and the just-clicked one on press.
const STATE_IDLE: u32 = 0;
const STATE_HOVER: u32 = 1;
const STATE_PRESS: u32 = 2;

/// `filetile.row_justify` declares `pack | fill`; the baked enum is the
/// word's index in that list.
const ROW_JUSTIFY_FILL: u32 = 1;

/// The engine's raw ink — what `theme_color` answers for a missing
/// token. Kept only for the path where the host predates ABI 5 and
/// cannot be asked at all.
const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
const NO_COLOR: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

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
    fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

// ------------------------------------------------------------------ theme

fn token(api: &HostApi, name: &str) -> u32 {
    (api.theme_token)(name.as_ptr(), name.len() as u32)
}

/// Token ids this widget draws from, resolved by NAME once per epoch.
///
/// No header tokens: `APPLICATIONS` and the count are the HOST's title
/// band, through `chrome`. This widget's drawing starts at the content
/// box's first row of tiles.
///
/// Two families meet here, and neither is this file's invention. The
/// GEOMETRY is `filetile.*` — the tile grid's own group, which the
/// theme documents as serving the launcher and the file browser alike
/// ("a launcher tile, a file tile" is how `class.tile` puts it). The
/// TYPE is `type.caption`, the role the theme describes in as many
/// words as "a launcher tile caption, FILES". Where a token for the
/// launcher grid does not exist at all, the file browser's is used and
/// said so in the report, never replaced by a number.
struct ThemeIds {
    epoch: u32,
    // form — the tile grid
    gap: u32,         // filetile.gap
    rows: u32,        // filetile.rows
    cols: u32,        // filetile.cols — a count, or the `auto` sentinel
    cell_min: u32,    // filetile.cell_min_px
    cell_pref: u32,   // filetile.cell — preferred tile edge; 0 = size from rows
    corner: u32,      // filetile.corner — the tile container's chamfer cut
    caption_gap: u32, // filetile.caption_gap
    icon_inset_x: u32, // filetile.icon.inset_x
    icon_inset_y: u32, // filetile.icon.inset_y
    icon_w: u32,      // filetile.icon.w
    icon_h: u32,      // filetile.icon.h
    wheel: u32,       // filetile.wheel_px
    row_justify: u32, // filetile.row_justify
    // the stand-in for the icon nobody can draw yet
    glyph_px: u32, // icon.size.launcher — "the launcher grid's app glyphs"
    // type — the launcher tile caption role
    caption_size: u32,     // type.caption.size
    caption_min: u32,      // type.caption.min_px
    caption_tracking: u32, // type.caption.tracking
    caption_leading: u32,  // type.caption.leading
    caption_case: u32,     // type.caption.case
    // where an empty grid says so
    empty_y: u32, // emptystate.y_frac
    // the press flash's life, and the one global that scales it
    press_ms: u32,     // motion.press.duration_ms
    motion_scale: u32, // motion.scale
    glow_scale: u32,   // glow.alpha_scale
    // the scrollbar
    sb_mode: u32,      // scrollbar.mode — `overlay | inset | none`; overlay = 0
    sb_w: u32,         // scrollbar.w
    sb_margin: u32,    // scrollbar.margin
    sb_thumb_min: u32, // scrollbar.thumb_min
    sb_side: u32,      // scrollbar.edge — `right | left`; right = 0
    /// The launcher tile's row in the class x state matrix.
    tile_class: u32,
    /// The scroll thumb's row in the same matrix.
    thumb_class: u32,
}

impl ThemeIds {
    fn resolve(api: &HostApi, epoch: u32) -> ThemeIds {
        let class = |name: &str| (api.theme_class)(name.as_ptr(), name.len() as u32);
        ThemeIds {
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
struct Look {
    idle: StateStyleC,
    hover: StateStyleC,
    press: StateStyleC,
    thumb: StateStyleC,
    gap: f32,
    rows: f32,
    cols: f32,
    cell_min: f32,
    cell_pref: f32,
    corner: f32,
    caption_gap: f32,
    icon_inset_x: f32,
    icon_inset_y: f32,
    icon_w: f32,
    icon_h: f32,
    wheel_px: f32,
    row_justify: u32,
    glyph_px: f32,
    caption_px: f32,
    caption_tracking: f32,
    caption_leading: f32,
    caption_case: u32,
    empty_y: f32,
    /// `motion.press.duration_ms` already scaled by `motion.scale` and
    /// turned into seconds — a reduced-motion theme sets the scale to 0
    /// and the flash simply never shows.
    press_s: f32,
    glow_scale: f32,
    sb_mode: u32,
    sb_w: f32,
    sb_margin: f32,
    sb_thumb_min: f32,
    sb_side: u32,
}

impl Look {
    /// The pre-token world: a host that answers no theme calls at all.
    /// Grey ink, zero lengths — the engine's own defaults, mirrored, so
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

    fn read(api: &HostApi, ctx: *mut c_void, t: &ThemeIds) -> Look {
        let px = |id| (api.theme_px)(ctx, id);
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
            idle: rung(t.tile_class, STATE_IDLE),
            hover: rung(t.tile_class, STATE_HOVER),
            press: rung(t.tile_class, STATE_PRESS),
            thumb: rung(t.thumb_class, STATE_IDLE),
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
) {
    // Every string this widget draws is centred on its tile, so the
    // alignment is not a parameter: 1 is the host's centre.
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
        1,
    );
}

// ----------------------------------------------------------- the widget

pub struct Appgrid {
    /// The installed applications, sorted by display name.
    entries: Vec<AppEntry>,
    /// Scroll offset in pixels; whole rows, like the file browser.
    pub scroll: f32,
    /// Tile rectangles from the last frame, for the hit test.
    hits: Vec<(Rect, usize)>,
    /// Which tile was clicked and when — WHICH state a tile is in is
    /// this file's to remember; how long the flash lasts is
    /// `motion.press.duration_ms`'s to say.
    pressed: Option<(usize, Instant)>,
    /// When the directories were last looked at, and what they said.
    last_look: Instant,
    stamp: u64,
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<ThemeIds>,
    /// `filetile.wheel_px`, cached at draw because a wheel event
    /// arrives with no drawing context to ask the theme through.
    wheel_px: f32,
    /// The count as last handed to the host's title band, alive until
    /// the next `chrome` call.
    chrome_right: Vec<u8>,
}

impl Appgrid {
    pub fn new() -> Self {
        let entries = desktop::scan();
        eprintln!("appgrid: {} applications found", entries.len());
        Appgrid {
            entries,
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: desktop::stamp(),
            theme: None,
            wheel_px: 0.0,
            chrome_right: Vec::new(),
        }
    }

    /// The menu, kept current without a thread and without a watch: a
    /// few `stat` calls no more than once every [`RESCAN_SECS`], and a
    /// walk only when they say something changed.
    fn follow(&mut self) {
        if self.last_look.elapsed().as_secs() < RESCAN_SECS {
            return;
        }
        self.last_look = Instant::now();
        let stamp = desktop::stamp();
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        self.entries = desktop::scan();
        eprintln!("appgrid: menu changed \u{2014} {} applications", self.entries.len());
    }

    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).max(0.0);
    }

    /// A click on a tile runs its application, detached. There is no
    /// action for the host to take: the widget owns this one itself,
    /// because `ActionC` has no code for "run this command" and
    /// inventing one would be an ABI change for one caller.
    pub fn click(&mut self, x: f32, y: f32) {
        let Some(idx) = self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|&(_, i)| i)
        else {
            return;
        };
        self.pressed = Some((idx, Instant::now()));
        let Some(app) = self.entries.get(idx) else { return };
        if let Err(e) = desktop::launch(app) {
            eprintln!("appgrid: {} \u{2014} {e}", app.name);
        }
    }

    /// This frame's theme values. Ids are cached across frames; the
    /// values are read fresh, because they are what a mood or a resize
    /// changes.
    fn look(&mut self, api: &HostApi, ctx: *mut c_void) -> Look {
        // ABI 5 is where the token entries live. attach() refuses an
        // older host outright, so this branch is belt and braces for the
        // day the check moves — an old table simply ends before these
        // entries do.
        if api.abi_version < 5 {
            return Look::raw();
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|t| t.epoch) != Some(epoch) {
            self.theme = Some(ThemeIds::resolve(api, epoch));
        }
        match &self.theme {
            Some(t) => Look::read(api, ctx, t),
            None => Look::raw(),
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        self.follow();
        let look = self.look(api, ctx);
        self.wheel_px = look.wheel_px;
        let name_px = look.caption_px;
        let name_sp = name_px * look.caption_tracking;

        if self.entries.is_empty() {
            // Nothing found is not an error — a machine can honestly
            // have no menu — so it says so in the caption role rather
            // than in a critical pill.
            let text = recase(look.caption_case, "no applications".to_string());
            // `emptystate.y_frac` says where in the box the line sits;
            // the role's own leading is what centres the line box on it
            // rather than hanging it below.
            draw_text(
                api,
                ctx,
                name_px,
                r.cx(),
                r.y + r.h * look.empty_y - name_px * look.caption_leading / 2.0,
                &text,
                look.idle.text,
                name_sp,
            );
            return;
        }

        // The tile grid; further rows reachable by scrolling. The same
        // arithmetic as the file browser's, over the same tokens.
        let area = Rect::new(r.x, r.y, r.w, r.h);
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
        // visible row ends exactly at the panel's bottom edge; pack sits
        // every row on filetile.gap.
        let step = if look.row_justify == ROW_JUSTIFY_FILL && total_rows > nvis && nvis > 1 {
            (area.h - tile) / (nvis as f32 - 1.0)
        } else {
            row_h
        };

        // Where the pointer is, once per frame; NaN matches no tile.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);
        // The press flash, once per frame as well: a click marks a tile
        // and the theme's own duration ends the mark.
        let flashing = self
            .pressed
            .filter(|(_, at)| at.elapsed().as_secs_f32() < look.press_s)
            .map(|(i, _)| i);

        for (i, app) in self.entries.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            if row < row_off || row >= row_off + nvis {
                continue;
            }
            let x = area.x + col as f32 * (tile + gap);
            let y = area.y + (row - row_off) as f32 * step;
            let trect = Rect::new(x, y, tile, tile);
            let rung = if flashing == Some(i) {
                &look.press
            } else if trect.contains(mx, my) {
                &look.hover
            } else {
                &look.idle
            };
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
                // Right after the stroke, the ring's glow — this rung's
                // own `glow_radius` and `glow_alpha` from the ladder,
                // tinted with the edge's resolved colour and scaled by
                // the one global knob. Every shipped idle rung is dark;
                // hover and press are where the ladder lights up.
                let alpha = (rung.glow_alpha * look.glow_scale).clamp(0.0, 1.0);
                if api.has_mask_quad() && rung.glow_radius > 0.0 && alpha > 0.0 {
                    let c = ColorC { a: alpha, ..rung.edge };
                    chamfer_glow(api, ctx, cell, cut, rung.glow_radius, c);
                }
            }

            // The stand-in for an icon: the application's initial, in
            // the launcher glyph size, centred in the box `filetile.icon.*`
            // reserves. Never taller than that box, so a squeezed grid
            // shrinks it instead of spilling it over the caption.
            let icon = Rect::new(
                x + tile * look.icon_inset_x,
                y + tile * look.icon_inset_y,
                tile * look.icon_w,
                tile * look.icon_h,
            );
            let mark = initial(&app.name);
            let gpx = look.glyph_px.min(icon.h).max(0.0);
            if !mark.is_empty() {
                draw_text(
                    api,
                    ctx,
                    gpx,
                    icon.cx(),
                    icon.cy() - gpx / 2.0,
                    &mark,
                    rung.glyph,
                    0.0,
                );
            }

            // The name under it, in the case the caption role asks for
            // and trimmed by measured width.
            let name = recase(look.caption_case, app.name.clone());
            let name = fit_name(api, ctx, name_px, &name, tile, name_sp);
            draw_text(
                api,
                ctx,
                name_px,
                trect.cx(),
                y + tile * look.caption_gap,
                &name,
                rung.text,
                name_sp,
            );

            self.hits.push((trect, i));
        }

        // The scrollbar, drawn at last: a panel that scrolls without
        // saying where it is is a defect. Overlay only — the word an
        // enum index can decode across the ABI; any other (inset, none)
        // draws nothing until the ABI can tell them apart.
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

impl Default for Appgrid {
    fn default() -> Self {
        Appgrid::new()
    }
}

/// The one character that stands in for an application's icon: the
/// first of its name, as a capital. A name that begins with something
/// uncased (a digit, a CJK ideograph) is left as it is, which is what
/// uppercasing means for those scripts anyway.
fn initial(name: &str) -> String {
    match name.chars().next() {
        Some(c) => c.to_uppercase().collect(),
        None => String::new(),
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
/// every `*.case` declares `enum: none | upper | lower | smallcaps`,
/// and `theme_enum` indexes that list. Smallcaps needs per-glyph sizes
/// only the host's font system has; through a single text call the
/// nearest honest reading is capitals.
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

/// Glow OUTSIDE the ring — the outline extruded outward by `radius`,
/// one additive quad per segment, the soft disk's cardinal strip laid
/// across the extrusion. Nothing is emitted inside the path, so the
/// glow never tints the fill.
fn chamfer_glow(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, radius: f32, c: ColorC) {
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

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Appgrid::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Appgrid) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Appgrid> {
    unsafe { (instance as *mut Appgrid).as_mut() }
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
        // The application is already on its way; the host has nothing
        // to do about it.
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

/// Grows downwards: a taller panel is more rows of applications, not
/// bigger ones. The width is what decides how big a tile is.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_ROWS
}

/// The header, as chrome: the panel's name on the left and how many
/// applications were found on the right — the same strings the title
/// band would have had to be told anyway, drawn once, trimmed once.
extern "C" fn chrome_c(
    instance: *mut c_void,
    _ctx: *mut c_void,
    _host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    static TITLE: &[u8] = b"APPLICATIONS";
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return 0;
    };
    this.chrome_right = this.entries.len().to_string().into_bytes();
    out.title = TITLE.as_ptr();
    out.title_len = TITLE.len() as u32;
    out.right = this.chrome_right.as_ptr();
    out.right_len = this.chrome_right.len() as u32;
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
