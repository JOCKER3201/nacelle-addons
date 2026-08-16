//! APPLICATIONS panel — the launcher grid: every installed XDG desktop
//! entry as a tile, alphabetical, clicking one runs it.
//!
//! It is the file browser's sibling by construction. The grid, the row
//! snapping, the wheel and the scrollbar are the same arithmetic over
//! the same `filetile.*` tokens, because a tile is a tile; what differs
//! is where the list comes from (see [`desktop`]) and what a click does
//! with it.
//!
//! WHICH applications it shows is not its own decision. The categories
//! list next door points it at one group or at the whole menu, and it
//! says so through the HOST — `nacelle::channel`, under
//! `selection::TOPIC`. This file is the READING end; read the head of
//! [`nacelle_launcher_core::selection`] before this one, because it says
//! why the choice cannot live in the crate the two widgets share.
//!
//! Pointed at the whole menu it draws the ALPHABETICAL INDEX: the tiles
//! broken into letter groups, each under its letter and a rule (see
//! [`sections`]). Pointed at one category it draws a flat page, because
//! an index over eleven applications is an index of nothing.
//!
//! THERE ARE NO ICONS, and that is a stated gap rather than an
//! oversight: the project has no icon theme, `Icon=` names a name in
//! one, and inventing artwork here would be a look this file decided
//! instead of the theme. Until the icon registry can answer, a tile
//! wears the initial of its application in the launcher glyph size —
//! neutral, legible, and nobody's design.
//!
//! Every colour, length, duration and word comes from the theme through
//! ABI 5/6 tokens. Nothing here knows what a colour is: a token nobody
//! can answer degrades to no ink and no length — nothing drawn — never
//! to a number that used to be the design.
//!
//! Almost none of the above is written here. The XDG scan, the
//! categories, the tile grid, the index and the selection are
//! `nacelle-launcher-core`, the launcher's shared half, because the
//! categories widget is built out of the same five and two copies of
//! any of them would be two launchers. What this file holds is the one
//! thing that is only the grid's: which applications the selection
//! admits, and what a click on a tile does.

use nacelle::runtime::{
    ActionC, ChromeC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_NONE,
};
use nacelle::widget::factory::BuiltinWidget;
use nacelle_launcher_core::desktop::AppEntry;
use nacelle_launcher_core::sections::{HeadLook, HeadTheme, Section};
use nacelle_launcher_core::selection::{Selection, Watch};
use nacelle_launcher_core::tile::{EmptyLook, EmptyTheme, Rect, TileLook, TileTheme};
use nacelle_launcher_core::{cats, desktop, sections, tile};
use std::ffi::c_void;
use std::time::Instant;

/// How often the menu is looked at again. The scan itself only runs
/// when the directories' modification times have MOVED, so this is the
/// rate of a handful of `stat` calls, not of a walk.
const RESCAN_SECS: u64 = 5;

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

// ----------------------------------------------------------- the widget

/// Both halves of this widget's look, read once per frame.
struct Look {
    tile: TileLook,
    head: HeadLook,
    /// The line this grid draws INSTEAD of its tiles. Its own look,
    /// because "the panel has nothing to show" is its own kind of
    /// element and the master answers it once, in `emptystate.role`.
    empty: EmptyLook,
}

pub struct Appgrid {
    /// The installed applications, sorted by display name.
    entries: Vec<AppEntry>,
    /// What the categories list has pointed this grid at, as of the
    /// last rebuild. A [`Watch`] so that a change can be NOTICED: the
    /// value on the host's board is written by another widget — in
    /// another `.so` — and nothing tells this one when.
    sel: Watch,
    /// Which of [`Appgrid::entries`] the selection admits, by index and
    /// in the scanner's order. Rebuilt when the selection or the menu
    /// moves, and not once per frame: the filter walks every entry's
    /// categories, which is a per-frame cost with no per-frame reason.
    view: Vec<usize>,
    /// The letter groups of [`Appgrid::view`] — empty unless the whole
    /// menu is on show, which is the only time an index is drawn.
    secs: Vec<Section>,
    /// Scroll offset in pixels; whole rows, like the file browser, or
    /// whole bands when the index is drawn.
    pub scroll: f32,
    /// Tile rectangles from the last frame, for the hit test. The index
    /// is into [`Appgrid::entries`] and never into the filtered view,
    /// so a rescan between a draw and a click cannot make a hit mean a
    /// different application than the one drawn.
    hits: Vec<(Rect, usize)>,
    /// Which tile was clicked and when — WHICH state a tile is in is
    /// this file's to remember; how long the flash lasts is
    /// `motion.press.duration_ms`'s to say.
    pressed: Option<(usize, Instant)>,
    /// When the directories were last looked at, and what they said.
    last_look: Instant,
    stamp: u64,
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<(TileTheme, HeadTheme, EmptyTheme)>,
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
        let mut this = Appgrid {
            entries,
            // A view that has not looked yet; the poll below reads
            // whatever is standing — ALL on a launcher nobody has
            // clicked, and the choice already made on one opened next to
            // a categories list that has been. The default lives in
            // `selection`, never in a copy here.
            sel: Watch::new(),
            view: Vec::new(),
            secs: Vec::new(),
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: desktop::stamp(),
            theme: None,
            wheel_px: 0.0,
            chrome_right: Vec::new(),
        };
        // Built at once for the choice already standing. NOT through
        // `refresh`: that one acts on a choice that MOVED, and reading
        // the standing one for the first time is not a move — a grid
        // that waited for one would have no page at all until somebody
        // clicked the list next door.
        this.sel.poll();
        let sel = this.sel.get().clone();
        this.rebuild(&sel);
        this
    }

    /// The menu, kept current without a thread and without a watch: a
    /// few `stat` calls no more than once every [`RESCAN_SECS`], and a
    /// walk only when they say something changed.
    ///
    /// Answers whether the menu MOVED, so the filtered view is rebuilt
    /// exactly when it has to be.
    fn follow(&mut self) -> bool {
        if self.last_look.elapsed().as_secs() < RESCAN_SECS {
            return false;
        }
        self.last_look = Instant::now();
        let stamp = desktop::stamp();
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        self.entries = desktop::scan();
        eprintln!("appgrid: menu changed \u{2014} {} applications", self.entries.len());
        true
    }

    /// The page this frame draws: which entries the selection admits,
    /// and where the letters break.
    ///
    /// Called at the top of every frame and cheap on almost all of
    /// them: the board is asked for a SEQUENCE NUMBER, and the page is
    /// only rebuilt in the frames where that number moved.
    fn refresh(&mut self) {
        if !self.sel.poll() {
            return;
        }
        let sel = self.sel.get().clone();
        self.rebuild(&sel);
        // A different selection is a different page, and a page starts
        // at its top. The scroll of the group looked at before it means
        // nothing here.
        self.scroll = 0.0;
    }

    /// The same view, rebuilt against the menu as it now is — what a
    /// rescan needs, and what must NOT move the scroll: the reader did
    /// not ask for a new page, an application was merely installed.
    fn rescan_view(&mut self) {
        let sel = self.sel.get().clone();
        self.rebuild(&sel);
    }

    fn rebuild(&mut self, sel: &Selection) {
        self.view = view_of(&self.entries, sel);
        // An index only when the whole menu is on show. A category's
        // own page is flat, and computing sections nobody draws would
        // be work done to be thrown away.
        self.secs = if sel.is_all() {
            sections::of(self.view.iter().map(|&i| self.entries[i].name.as_str()))
        } else {
            Vec::new()
        };
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
            return Look {
                tile: TileLook::raw(),
                head: HeadLook::raw(),
                empty: EmptyLook::raw(),
            };
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|(t, _, _)| t.epoch) != Some(epoch) {
            self.theme = Some((
                TileTheme::resolve(api, ctx, epoch),
                HeadTheme::resolve(api, ctx, epoch),
                EmptyTheme::resolve(api, ctx, epoch),
            ));
        }
        match &self.theme {
            Some((t, h, e)) => {
                debug_assert_eq!(t.epoch, h.epoch);
                debug_assert_eq!(t.epoch, e.epoch);
                Look {
                    tile: TileLook::read(api, ctx, t),
                    head: HeadLook::read(api, ctx, h),
                    empty: EmptyLook::read(api, ctx, e),
                }
            }
            None => Look {
                tile: TileLook::raw(),
                head: HeadLook::raw(),
                empty: EmptyLook::raw(),
            },
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        if self.follow() {
            self.rescan_view();
        }
        self.refresh();
        let look = self.look(api, ctx);
        self.wheel_px = look.tile.wheel_px;

        if self.view.is_empty() {
            // Nothing found is not an error — a machine can honestly
            // have no menu, and a category can honestly be emptied by
            // the uninstall that happened between two frames — so it
            // says so in a line rather than in a critical pill.
            //
            // In `emptystate.role`, which is the master's answer for
            // every panel that has nothing to show. It used to be the
            // tile CAPTION's role, which drew this sentence at 9.6 px
            // while the categories list beside it drew the same sentence
            // at 13.3 px and the search panel drew it at 17.6 px.
            tile::empty_line(
                api,
                ctx,
                &look.empty,
                r,
                "no applications",
                // The grid's own resting ink, so the line reads as part
                // of the surface the tiles would have covered.
                Some(look.tile.idle.text),
            );
            return;
        }

        let area = Rect::new(r.x, r.y, r.w, r.h);
        // Where the pointer is, once per frame; NaN matches no tile.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);
        // The press flash, once per frame as well: a click marks a tile
        // and the theme's own duration ends the mark.
        let flashing = self
            .pressed
            .filter(|(_, at)| at.elapsed().as_secs_f32() < look.tile.press_s)
            .map(|(i, _)| i);
        let p = Pointer { x: mx, y: my, flashing };

        let scroll = if self.secs.is_empty() {
            self.draw_flat(api, ctx, area, &look, p)
        } else {
            self.draw_index(api, ctx, area, &look, p)
        };

        // The scrollbar, drawn at last: a panel that scrolls without
        // saying where it is is a defect. Rows or bands, the bar reads
        // the same four numbers.
        tile::scrollbar(api, ctx, &look.tile, area, scroll);
    }

    /// One category's applications: the tile grid, flat, further rows
    /// reachable by scrolling. The same arithmetic as the file
    /// browser's, over the same tokens.
    fn draw_flat(
        &mut self,
        api: &HostApi,
        ctx: *mut c_void,
        area: Rect,
        look: &Look,
        p: Pointer,
    ) -> tile::Scroll {
        let grid = tile::layout(&look.tile, area, self.view.len(), &mut self.scroll);
        for (n, &i) in self.view.iter().enumerate() {
            let Some(trect) = grid.place(area, n) else { continue };
            let Some(app) = self.entries.get(i) else { continue };
            self.hits.push((trect, i));
            tile::tile_face(
                api,
                ctx,
                &look.tile,
                trect,
                p.rung(&look.tile, trect, i),
                &tile::initial(&app.name),
                &app.name,
            );
        }
        grid.scroll()
    }

    /// The whole menu: the same tiles, broken into letter groups, each
    /// under its letter and the rule beneath it.
    fn draw_index(
        &mut self,
        api: &HostApi,
        ctx: *mut c_void,
        area: Rect,
        look: &Look,
        p: Pointer,
    ) -> tile::Scroll {
        let head_h = look.head.height(look.tile.gap);
        let plan = sections::plan(&look.tile, head_h, area, &self.secs, &mut self.scroll);
        // Collected because drawing borrows `self.entries` and pushing
        // a hit borrows `self.hits`, and the plan borrows neither.
        let bands: Vec<(Option<char>, usize, usize, f32)> =
            plan.visible(area).map(|(b, y)| (b.key, b.first, b.count, y)).collect();
        for (key, first, count, y) in bands {
            if let Some(key) = key {
                sections::head(api, ctx, &look.head, area, y, key);
                continue;
            }
            for n in 0..count {
                let trect = plan.cell(area, n, y);
                let Some(&i) = self.view.get(first + n) else { continue };
                let Some(app) = self.entries.get(i) else { continue };
                self.hits.push((trect, i));
                tile::tile_face(
                    api,
                    ctx,
                    &look.tile,
                    trect,
                    p.rung(&look.tile, trect, i),
                    &tile::initial(&app.name),
                    &app.name,
                );
            }
        }
        plan.scroll()
    }
}

/// Which entries a selection admits, by index and in the order they
/// were given in.
///
/// A free function and not a method, because it is the whole of what
/// "the categories list steers the grid" MEANS and it holds no state:
/// the whole menu admits everything, a group admits what [`cats::holds`]
/// says is in it, and a group nothing is in admits nothing — which is
/// not an error but a category emptied by an uninstall.
fn view_of(entries: &[AppEntry], sel: &Selection) -> Vec<usize> {
    match sel.name() {
        None => (0..entries.len()).collect(),
        Some(name) => entries
            .iter()
            .enumerate()
            .filter(|(_, e)| cats::holds(name, e))
            .map(|(i, _)| i)
            .collect(),
    }
}

/// Everything the state ladder needs to know about this frame's
/// pointer, so that picking a rung is one call rather than the same
/// three-armed `if` at two drawing sites.
#[derive(Clone, Copy)]
struct Pointer {
    x: f32,
    y: f32,
    flashing: Option<usize>,
}

impl Pointer {
    /// Which rung of `class.tile` the tile in `r` is resting on: idle,
    /// hover under the pointer, press for as long as the theme's own
    /// duration says a click still shows.
    fn rung<'a>(&self, look: &'a TileLook, r: Rect, i: usize) -> &'a StateStyleC {
        if self.flashing == Some(i) {
            &look.press
        } else if r.contains(self.x, self.y) {
            &look.hover
        } else {
            &look.idle
        }
    }
}

impl Default for Appgrid {
    fn default() -> Self {
        Appgrid::new()
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
/// applications are ON SHOW on the right — the same strings the title
/// band would have had to be told anyway, drawn once, trimmed once.
///
/// On show, not installed: the number over a filtered page has to be
/// the number of that page, or the count and the tiles under it
/// contradict each other. With the whole menu selected the two are the
/// same figure, which is what the panel showed before it could be
/// filtered at all.
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
    // `chrome` may be asked before the first frame, so the view is
    // brought level here too rather than only at draw.
    this.refresh();
    this.chrome_right = this.view.len().to_string().into_bytes();
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

/// Filled, and consumes nothing on purpose: this grid is walked with the
/// pointer and has no keyboard cursor to move. Answering 0 leaves every
/// key with the host, which is where the focus chain and the shortcuts
/// are — a grid that swallowed Tab would trap the keyboard in a panel
/// that cannot use it.
extern "C" fn key_c(
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
/// carries is one this panel already draws from its own clock — a tile
/// marks itself for `motion.press.duration_ms` from the click — so
/// taking the press here as well would be a second source of one state,
/// and the two would disagree the first time a press was released
/// somewhere else.
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
/// `appgrid.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `appgrid.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "appgrid",
    meta: include_str!("../appgrid.meta"),
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
mod token_tests {
    /// Every token name this crate asks the theme for, and every class
    /// name, spelled exactly as the code spells it.
    ///
    /// The test below is the one that makes "no hardcoded values" a
    /// FACT rather than a promise. A widget that names a token the
    /// master does not declare gets `u32::MAX` back, `theme_px` answers
    /// zero, and the thing degrades silently: a separator of no width
    /// and a letter of no size look exactly like a section index nobody
    /// implemented. A typo would therefore never fail loudly anywhere
    /// else — so it fails here.
    const TOKENS: &[&str] = &[
        // filetile.* — the tile grid, shared with the file browser
        "filetile.gap",
        "filetile.rows",
        "filetile.cols",
        "filetile.cell_min_px",
        "filetile.cell",
        "filetile.corner",
        "filetile.caption_gap",
        "filetile.icon.inset_x",
        "filetile.icon.inset_y",
        "filetile.icon.w",
        "filetile.icon.h",
        "filetile.wheel_px",
        "filetile.row_justify",
        "icon.size.launcher",
        // The caption's BINDING. The role's own family is not listed
        // here — it is spelled `type.<word>.*` at run time, and the
        // test below chases the word the master actually binds.
        "tile.caption_role",
        "emptystate.y_frac",
        "motion.press.duration_ms",
        "motion.scale",
        "glow.alpha_scale",
        "scrollbar.mode",
        "scrollbar.w",
        "scrollbar.margin",
        "scrollbar.thumb_min",
        "scrollbar.edge",
        // the alphabetical index's heading — see `sections::HeadTheme`
        "type.label.section.size",
        "type.label.section.min_px",
        "type.label.section.tracking",
        "type.label.section.leading",
        "type.label.section.case",
        "type.label.section.face",
        "type.label.section.fg",
        "space.2",
        "table.rule",
        "component.table.rule",
    ];

    /// Nothing is excused any more: the master's missing `[emptystate]`
    /// header was the one entry this list ever carried, and it landed.
    const MAY_BE_MISSING: &[&str] = &[];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let mut missing: Vec<&str> = Vec::new();
        for name in TOKENS {
            if nacelle::theme::id(name).is_none() && !MAY_BE_MISSING.contains(name) {
                missing.push(name);
            }
        }
        assert!(missing.is_empty(), "the master declares no {missing:?}");
        // The classes the two widgets rest on, by the same argument: a
        // class the matrix does not know answers the raw rung, and the
        // grid would look undesigned for a reason no reader could see.
        for class in ["tile", "scrollbar.thumb", "list.item"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
        // And the rungs the categories list needs, which are the ones
        // this change added a use for.
        assert!(nacelle::theme::id("state.selected.fill").is_some());
        assert!(nacelle::theme::id("state.selected_hover.fill").is_some());
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;

    /// A menu as the scanner would hand it over: sorted by the
    /// lowercased display name, which is what the alphabetical index
    /// is entitled to assume of its input.
    fn menu() -> Vec<AppEntry> {
        let e = |name: &str, categories: &[&str]| AppEntry {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.to_string(),
            exec: "/bin/true".to_string(),
            terminal: false,
            icon: String::new(),
            categories: categories.iter().map(|s| s.to_string()).collect(),
        };
        let mut v = vec![
            e("0 A.D.", &["Game"]),
            e("7-Zip", &["Utility", "Archiving"]),
            e("Ark", &["Utility", "Qt"]),
            e("Audacity", &["AudioVideo", "Audio"]),
            e("Blender", &["Graphics"]),
            e("Toolkit", &["Qt", "KDE"]),
            e("Łoś", &["Game"]),
            e("Ósemka", &["Utility"]),
            e("Żaba", &["Game"]),
        ];
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    #[test]
    fn all_applications_shows_exactly_what_the_scanner_found() {
        let m = menu();
        let view = view_of(&m, &Selection::All);
        // Exactly as many as there are, in the order they came in, and
        // no entry reached twice — the counts on the list's top row and
        // the tiles on the grid are the same statement.
        assert_eq!(view.len(), m.len());
        assert_eq!(view, (0..m.len()).collect::<Vec<_>>());
        // Including the entries no category places, which the grid must
        // never hide: a launcher that loses an installed application is
        // worse than an untidy one.
        assert!(m.iter().any(|e| cats::main_categories(&e.categories).is_empty()));
        assert_eq!(view_of(&[], &Selection::All), Vec::<usize>::new());
    }

    #[test]
    fn a_chosen_category_shows_that_category_and_switching_switches() {
        let m = menu();
        let named = |n: &str| {
            view_of(&m, &Selection::Named(n.to_string()))
                .into_iter()
                .map(|i| m[i].name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(named("Game"), ["0 A.D.", "Łoś", "Żaba"]);
        assert_eq!(named("Utility"), ["7-Zip", "Ark", "Ósemka"]);
        assert_eq!(named("Graphics"), ["Blender"]);
        // One entry, two memberships: choosing either shows it.
        assert_eq!(named("AudioVideo"), ["Audacity"]);
        assert_eq!(named("Audio"), ["Audacity"]);
        // What no main category placed is reachable under Other, and
        // only there.
        assert_eq!(named(cats::OTHER), ["Toolkit"]);
        // Switching is a whole different page and not a filter over the
        // last one: nothing of Game survives into Graphics.
        assert!(named("Graphics").iter().all(|n| !named("Game").contains(n)));
        // A category the menu no longer has — the last application in
        // it uninstalled while it was the chosen one — is an empty
        // page, not a stale one and not a panic.
        assert!(named("Science").is_empty());
        assert!(named("NotACategory").is_empty());
        assert!(view_of(&[], &Selection::Named("Game".to_string())).is_empty());
    }

    #[test]
    fn the_index_over_the_whole_menu_covers_it_letter_by_letter() {
        let m = menu();
        let view = view_of(&m, &Selection::All);
        let secs = sections::of(view.iter().map(|&i| m[i].name.as_str()));
        let keys: Vec<char> = secs.iter().map(|s| s.key).collect();
        // The digits share one group; every letter has its own,
        // diacritics included and never folded into A..Z.
        //
        // The diacritics come after `T` and in THAT order because the
        // scanner sorts by code point and `Ó` < `Ł` < `Ż` there. The
        // index does not correct it — see `sections::of`: fixing the
        // ORDER is the scanner's comparison, and an index that re-sorted
        // its own input would be labelling a page it had rearranged.
        assert_eq!(keys, [sections::NON_LETTER, 'A', 'B', 'T', 'Ó', 'Ł', 'Ż']);
        // Each of the three is its own heading over its own application.
        for (k, want) in [('Ó', "Ósemka"), ('Ł', "Łoś"), ('Ż', "Żaba")] {
            let s = secs.iter().find(|s| s.key == k).unwrap();
            assert_eq!((s.len, m[view[s.start]].name.as_str()), (1, want));
        }
        // Every application is under exactly one letter, and the groups
        // are the page in order.
        assert_eq!(secs.iter().map(|s| s.len).sum::<usize>(), view.len());
        let first = &secs[0];
        assert_eq!(first.len, 2, "0 A.D. and 7-Zip share the one non-letter group");
        // A chosen category is drawn flat: no index is computed for it
        // at all, which is what `Appgrid::rebuild` checks by asking the
        // selection rather than by counting sections.
        assert!(!Selection::Named("Game".to_string()).is_all());
        assert!(Selection::All.is_all());
    }

    /// A grid over a menu of this file's own, without the XDG scan
    /// `Appgrid::new` does: a test whose page depended on what happens
    /// to be installed would be a different test on every machine.
    fn grid_over(entries: Vec<AppEntry>) -> Appgrid {
        let mut g = Appgrid {
            entries,
            sel: Watch::new(),
            view: Vec::new(),
            secs: Vec::new(),
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: 0,
            theme: None,
            wheel_px: 0.0,
            chrome_right: Vec::new(),
        };
        g.sel.poll();
        let sel = g.sel.get().clone();
        g.rebuild(&sel);
        g
    }

    /// ONE test that touches the board, not four: it is process-wide
    /// state and two tests writing it would race each other under the
    /// default harness — `launcher-core`'s own channel test says the
    /// same and for the same reason.
    ///
    /// What this one proves is the half `launcher-core` cannot: that the
    /// GRID's page, index and scroll all follow a choice this crate
    /// never sees made. Nothing but the host's board passes between the
    /// two ends.
    #[test]
    fn the_page_follows_the_choice_the_categories_panel_published() {
        let m = menu();
        let mut grid = grid_over(menu());
        // Nobody has chosen yet: the whole menu, under its letter index.
        assert_eq!(grid.view.len(), m.len());
        assert!(!grid.secs.is_empty(), "the whole menu is drawn with its index");

        // The categories panel is another widget in another `.so` with
        // its own copy of this crate; a `Watch` of its own is the
        // closest a single test binary can stand to one.
        let mut cats_panel = Watch::new();
        cats_panel.set(Selection::Named("Game".to_string()));
        grid.scroll = 120.0;
        grid.refresh();
        let names: Vec<&str> = grid.view.iter().map(|&i| m[i].name.as_str()).collect();
        assert_eq!(names, ["0 A.D.", "Łoś", "Żaba"]);
        assert!(grid.secs.is_empty(), "a category's own page is flat");
        assert_eq!(grid.scroll, 0.0, "a different page starts at its top");

        // And a frame in which nothing was published rebuilds nothing —
        // the sequence number is what makes that cheap, and the scroll
        // standing still is what makes it visible.
        grid.scroll = 40.0;
        grid.refresh();
        assert_eq!(grid.scroll, 40.0);

        // The top row takes it back, index and all.
        cats_panel.set(Selection::All);
        grid.refresh();
        assert_eq!(grid.view.len(), m.len());
        assert!(!grid.secs.is_empty());

        // A grid created after the click reads the choice standing,
        // which is what makes load order stop mattering.
        cats_panel.set(Selection::Named("Utility".to_string()));
        let late = grid_over(menu());
        let names: Vec<&str> = late.view.iter().map(|&i| m[i].name.as_str()).collect();
        assert_eq!(names, ["7-Zip", "Ark", "Ósemka"]);

        // Put the board back, so the order tests run in cannot matter.
        cats_panel.set(Selection::All);
    }
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
    /// above `key_c` and `button_c`, and a later change that gave
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
