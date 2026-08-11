//! CATEGORIES panel — the same installed applications as the launcher
//! grid, entered through the groups their desktop entries put them in.
//!
//! It is the grid's sister, not its copy. The menu is found by the
//! grid's own XDG scanner, a tile is drawn by the grid's own tile, and
//! a click starts an application through the grid's own detached
//! launch; what this widget adds is one question the grid does not ask
//! — *which group is this in* — and the two-level view that answers it.
//! Everything about a desktop entry that could be parsed twice in this
//! tree is parsed once, in [`nacelle_widget_appgrid::desktop`].
//!
//! Two views, one panel. The first is the list of groups this machine
//! actually has, alphabetically, each with how many applications it
//! holds. Clicking one opens it: a row that names where you are and
//! takes you back, and under it the applications as tiles. Which group
//! is open, and how far each view is scrolled, belong to the widget
//! INSTANCE — two of these panels on two boards are two independent
//! places to be.
//!
//! Every colour, length, duration and word comes from the theme through
//! ABI 5/6 tokens. Nothing here knows what a colour is: a missing token
//! degrades through the raw answers the ABI itself gives (grey ink,
//! zero lengths), never through a number that used to be the design.

pub mod cats;

use cats::Category;
use nacelle::runtime::{
    ActionC, ChromeC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_NONE,
};
use nacelle_widget_appgrid::desktop::{self, AppEntry};
use nacelle_widget_appgrid::tile::{self, Rect, TileLook, TileTheme};
use std::ffi::c_void;
use std::time::Instant;

/// How often the menu is looked at again — the grid's own rate, for the
/// same reason: the scan only runs when the directories' modification
/// times have MOVED, so this is a handful of `stat` calls.
const RESCAN_SECS: u64 = 5;

/// The mark on the row that leaves a category. A chevron rather than a
/// word, because the row already carries a word — the category's name —
/// and two would be one too many at this size.
const BACK_MARK: &str = "\u{2039}";

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

// ------------------------------------------------------------------ theme

/// Token ids the ROW half of this widget draws from — the half the tile
/// grid has no opinion about. The tiles' own ids are
/// [`TileTheme`]'s, resolved beside these and never duplicated.
///
/// The family is `list.*`, the theme's own group for "a task,
/// notification or process row", and the class is `list.item`, which
/// the state matrix describes in exactly those words. Where the list
/// group declares nothing for something a row needs, the tile grid's
/// own token is used rather than a number, and the gap is reported.
struct ListTheme {
    epoch: u32,
    row_h: u32,      // list.row_h
    row_gap: u32,    // list.gap
    pad_x: u32,      // list.pad_x
    chip: u32,       // list.glyph — the leading coloured chip of a row
    chip_gap: u32,   // list.glyph_gap
    status_gap: u32, // list.status_gap — the gap before a row's trailing status
    // type — the count is a number in a column, which is `data`'s role
    count_size: u32,    // type.data.size
    count_min: u32,     // type.data.min_px
    count_tracking: u32, // type.data.tracking
    count_leading: u32, // type.data.leading
    count_case: u32,    // type.data.case
    /// The font slots `type.data.face` and `type.caption.face` name,
    /// resolved WITH the ids because a face is a word and reading words
    /// is init-time work. `data` asks for the monospace face and means
    /// it: a column of counts lines up because the figures are tabular,
    /// which is a property of the face and not of this file.
    count_font: u32,
    label_font: u32,
    /// Where an empty panel says so. `emptystate.y_frac` is the name the
    /// master MEANS — its own comment reads "where that message sits in
    /// the empty box" — but the two keys sit under `[boot]` in
    /// `default.theme` and so are addressable only as `boot.y_frac`.
    /// Asked for by the right name first, so the day the section header
    /// lands this widget follows it without an edit.
    empty_y: u32,
    /// A row's row in the class x state matrix.
    item_class: u32,
}

impl ListTheme {
    fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> ListTheme {
        let mut empty_y = tile::token(api, "emptystate.y_frac");
        if empty_y == u32::MAX {
            empty_y = tile::token(api, "boot.y_frac");
        }
        ListTheme {
            epoch,
            row_h: tile::token(api, "list.row_h"),
            row_gap: tile::token(api, "list.gap"),
            pad_x: tile::token(api, "list.pad_x"),
            chip: tile::token(api, "list.glyph"),
            chip_gap: tile::token(api, "list.glyph_gap"),
            status_gap: tile::token(api, "list.status_gap"),
            count_size: tile::token(api, "type.data.size"),
            count_min: tile::token(api, "type.data.min_px"),
            count_tracking: tile::token(api, "type.data.tracking"),
            count_leading: tile::token(api, "type.data.leading"),
            count_case: tile::token(api, "type.data.case"),
            count_font: tile::face_slot(api, ctx, tile::token(api, "type.data.face")),
            label_font: tile::face_slot(api, ctx, tile::token(api, "type.caption.face")),
            empty_y,
            item_class: (api.theme_class)("list.item".as_ptr(), "list.item".len() as u32),
        }
    }
}

/// The row values one frame draws with, read fresh from the resolved
/// ids. Colours and lengths only — nothing here is arithmetic on
/// anything.
struct ListLook {
    idle: StateStyleC,
    hover: StateStyleC,
    press: StateStyleC,
    row_h: f32,
    row_gap: f32,
    pad_x: f32,
    chip: f32,
    chip_gap: f32,
    status_gap: f32,
    count_px: f32,
    count_tracking: f32,
    count_leading: f32,
    count_case: u32,
    count_font: u32,
    label_font: u32,
    empty_y: f32,
}

impl ListLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// Zero lengths, the matrix's own raw ink — an old host shows the
    /// same undesigned raw as an empty theme.
    fn raw() -> ListLook {
        let raw_state = StateStyleC {
            fill: tile::NO_COLOR,
            edge: tile::RAW_INK,
            text: tile::RAW_INK,
            glyph: tile::RAW_INK,
            edge_width: 1.0,
            glow_radius: 0.0,
            glow_alpha: 0.0,
            elevation: 0.0,
        };
        ListLook {
            idle: raw_state,
            hover: raw_state,
            press: raw_state,
            row_h: 0.0,
            row_gap: 0.0,
            pad_x: 0.0,
            chip: 0.0,
            chip_gap: 0.0,
            status_gap: 0.0,
            count_px: 0.0,
            count_tracking: 0.0,
            count_leading: 1.0,
            count_case: 0,
            count_font: tile::FONT_UI,
            label_font: tile::FONT_UI,
            empty_y: 0.0,
        }
    }

    fn read(api: &HostApi, ctx: *mut c_void, t: &ListTheme) -> ListLook {
        let px = |id| (api.theme_px)(ctx, id);
        ListLook {
            idle: tile::rung(api, ctx, t.item_class, tile::STATE_IDLE),
            hover: tile::rung(api, ctx, t.item_class, tile::STATE_HOVER),
            press: tile::rung(api, ctx, t.item_class, tile::STATE_PRESS),
            row_h: px(t.row_h),
            row_gap: px(t.row_gap),
            pad_x: px(t.pad_x),
            chip: px(t.chip),
            chip_gap: px(t.chip_gap),
            status_gap: px(t.status_gap),
            count_px: px(t.count_size).max(px(t.count_min)),
            count_tracking: px(t.count_tracking),
            count_leading: px(t.count_leading).max(1.0),
            count_case: (api.theme_enum)(ctx, t.count_case),
            count_font: t.count_font,
            label_font: t.label_font,
            empty_y: px(t.empty_y),
        }
    }
}

/// Both halves of this widget's look, read once per frame.
struct Look {
    tile: TileLook,
    list: ListLook,
}

// ----------------------------------------------------------- the widget

/// What the pointer can be over. The press flash and the click path
/// both speak in these rather than in indices, so "the third row"
/// cannot be mistaken for "the third tile" across a view change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// The row that leaves the open category.
    Back,
    /// A group in the list, by its place in [`Appcats::cats`].
    Cat(usize),
    /// An application in the open group, by its place in
    /// [`Appcats::entries`].
    App(usize),
}

pub struct Appcats {
    /// The installed applications, sorted by display name — the grid's
    /// own list, scanned by the grid's own scanner.
    entries: Vec<AppEntry>,
    /// The groups those entries fall into, alphabetically.
    cats: Vec<Category>,
    /// Which group is open, BY NAME. A name rather than an index
    /// because a rescan rebuilds the groups: installing a game where
    /// there were none moves every group after it by one, and an index
    /// would quietly become a different category under the reader.
    open: Option<&'static str>,
    /// Scroll offset in pixels for each view, kept apart so that going
    /// back lands where you left rather than at the top.
    scroll_cats: f32,
    scroll_apps: f32,
    /// What was where in the last frame, for the hit test.
    hits: Vec<(Rect, Hit)>,
    /// What was clicked and when — WHICH state a thing is in is this
    /// file's to remember; how long the flash lasts is
    /// `motion.press.duration_ms`'s to say.
    pressed: Option<(Hit, Instant)>,
    /// When the directories were last looked at, and what they said.
    last_look: Instant,
    stamp: u64,
    /// Resolved token ids, re-resolved whenever the theme epoch moves.
    theme: Option<(TileTheme, ListTheme)>,
    /// `filetile.wheel_px`, cached at draw because a wheel event
    /// arrives with no drawing context to ask the theme through.
    wheel_px: f32,
    /// The count as last handed to the host's title band, alive until
    /// the next `chrome` call.
    chrome_right: Vec<u8>,
}

impl Appcats {
    pub fn new() -> Self {
        let entries = desktop::scan();
        let cats = cats::group(&entries);
        eprintln!(
            "appcats: {} categories over {} applications",
            cats.len(),
            entries.len()
        );
        Appcats {
            entries,
            cats,
            open: None,
            scroll_cats: 0.0,
            scroll_apps: 0.0,
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
        self.cats = cats::group(&self.entries);
        eprintln!(
            "appcats: menu changed \u{2014} {} categories over {} applications",
            self.cats.len(),
            self.entries.len()
        );
    }

    /// Where the open category is in the current grouping, or None when
    /// nothing is open — or when what was open no longer exists,
    /// because the last application in it was uninstalled while it was
    /// being looked at. Being returned to the list is the honest
    /// answer to that; an empty group would be a lie about the machine.
    fn open_index(&self) -> Option<usize> {
        let want = self.open?;
        self.cats.iter().position(|c| c.name == want)
    }

    pub fn wheel(&mut self, delta: f32) {
        let s = if self.open.is_some() { &mut self.scroll_apps } else { &mut self.scroll_cats };
        *s = (*s - delta).max(0.0);
    }

    /// A click on a group opens it, a click on the back row closes it,
    /// and a click on a tile runs its application, detached. There is
    /// no action for the host to take in any of the three: the two
    /// navigations are this widget's own state, and `ActionC` has no
    /// code for "run this command" — inventing one would be an ABI
    /// change for one caller.
    pub fn click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|&(_, h)| h)
        else {
            return;
        };
        self.pressed = Some((hit, Instant::now()));
        match hit {
            Hit::Back => {
                self.open = None;
            }
            Hit::Cat(i) => {
                let Some(c) = self.cats.get(i) else { return };
                self.open = Some(c.name);
                // A group is entered at its top, always: the scroll of
                // the group looked at before it means nothing here.
                self.scroll_apps = 0.0;
            }
            Hit::App(i) => {
                let Some(app) = self.entries.get(i) else { return };
                if let Err(e) = desktop::launch(app) {
                    eprintln!("appcats: {} \u{2014} {e}", app.name);
                }
            }
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
            return Look { tile: TileLook::raw(), list: ListLook::raw() };
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|(t, _)| t.epoch) != Some(epoch) {
            self.theme =
                Some((TileTheme::resolve(api, epoch), ListTheme::resolve(api, ctx, epoch)));
        }
        match &self.theme {
            Some((t, l)) => {
                debug_assert_eq!(t.epoch, l.epoch);
                Look { tile: TileLook::read(api, ctx, t), list: ListLook::read(api, ctx, l) }
            }
            None => Look { tile: TileLook::raw(), list: ListLook::raw() },
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        self.follow();
        let look = self.look(api, ctx);
        self.wheel_px = look.tile.wheel_px;

        // Where the pointer is, once per frame; NaN matches nothing.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);
        // The press flash, once per frame as well: a click marks a
        // thing and the theme's own duration ends the mark.
        let flashing = self
            .pressed
            .filter(|(_, at)| at.elapsed().as_secs_f32() < look.tile.press_s)
            .map(|(h, _)| h);
        let pointer = Pointer { x: mx, y: my, flashing };

        match self.open_index() {
            Some(ci) => self.draw_apps(api, ctx, r, &look, ci, pointer),
            None => self.draw_cats(api, ctx, r, &look, pointer),
        }
    }

    /// The list of groups: one row each, alphabetical, the count of
    /// applications on the right.
    fn draw_cats(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect, look: &Look, p: Pointer) {
        if self.cats.is_empty() {
            empty(api, ctx, r, look, "no applications");
            return;
        }
        let pitch = look.list.row_h + look.list.row_gap;
        let s = rows(pitch, look.list.row_gap, r.h, self.cats.len(), &mut self.scroll_cats);

        for (i, c) in self.cats.iter().enumerate().skip(s.off).take(s.nvis) {
            let rect =
                Rect::new(r.x, r.y + (i - s.off) as f32 * pitch, r.w, look.list.row_h);
            let rung = p.rung(look.rows(), rect, Hit::Cat(i));
            row(
                api,
                ctx,
                look,
                rect,
                rung,
                &tile::initial(c.name),
                c.name,
                &c.apps.len().to_string(),
            );
            self.hits.push((rect, Hit::Cat(i)));
        }
        tile::scrollbar(api, ctx, &look.tile, r, s);
    }

    /// One group's applications: the row that says where you are and
    /// takes you back, and the grid's own tiles under it.
    fn draw_apps(
        &mut self,
        api: &HostApi,
        ctx: *mut c_void,
        r: Rect,
        look: &Look,
        ci: usize,
        p: Pointer,
    ) {
        let cat = &self.cats[ci];
        // The way out is drawn FIRST and unconditionally, whatever is
        // left for the tiles: a panel squeezed too short to show one
        // application must still be a panel you can leave.
        let back = Rect::new(r.x, r.y, r.w, look.list.row_h);
        let rung = p.rung(look.rows(), back, Hit::Back);
        row(
            api,
            ctx,
            look,
            back,
            rung,
            BACK_MARK,
            cat.name,
            &cat.apps.len().to_string(),
        );
        self.hits.push((back, Hit::Back));

        let top = look.list.row_h + look.list.row_gap;
        let area = Rect::new(r.x, r.y + top, r.w, r.h - top);
        if area.h <= 0.0 {
            return;
        }
        if cat.apps.is_empty() {
            // Unreachable through the list, which only offers groups
            // that hold something — but a rescan between the click and
            // this frame can empty one, and an empty box that says
            // nothing is a defect whatever led to it.
            empty(api, ctx, area, look, "no applications");
            return;
        }
        let grid = tile::layout(&look.tile, area, cat.apps.len(), &mut self.scroll_apps);
        for (n, &i) in cat.apps.iter().enumerate() {
            let Some(trect) = grid.place(area, n) else { continue };
            let Some(app) = self.entries.get(i) else { continue };
            let rung = p.rung(look.tiles(), trect, Hit::App(i));
            tile::tile_face(
                api,
                ctx,
                &look.tile,
                trect,
                rung,
                &tile::initial(&app.name),
                &app.name,
            );
            self.hits.push((trect, Hit::App(i)));
        }
        tile::scrollbar(api, ctx, &look.tile, area, grid.scroll());
    }
}

impl Default for Appcats {
    fn default() -> Self {
        Appcats::new()
    }
}

/// Everything the state ladder needs to know about this frame's
/// pointer, so that picking a rung is one call rather than the same
/// three-armed `if` at every drawing site.
#[derive(Clone, Copy)]
struct Pointer {
    x: f32,
    y: f32,
    flashing: Option<Hit>,
}

impl Pointer {
    /// Which rung of a ladder the thing in `r` is resting on this
    /// frame. A container is on its class's idle rung, the pointed-at
    /// one on hover, and the just-clicked one on press — the same three
    /// for a row and for a tile, which is why the ladder is a parameter
    /// and not two copies of this function.
    fn rung<'a>(&self, l: Ladder<'a>, r: Rect, what: Hit) -> &'a StateStyleC {
        if self.flashing == Some(what) {
            l.press
        } else if r.contains(self.x, self.y) {
            l.hover
        } else {
            l.idle
        }
    }
}

/// The three rungs of one class's state ladder that this widget uses.
#[derive(Clone, Copy)]
struct Ladder<'a> {
    idle: &'a StateStyleC,
    hover: &'a StateStyleC,
    press: &'a StateStyleC,
}

impl Look {
    /// `class.list.item` — the ladder a row rests on.
    fn rows(&self) -> Ladder<'_> {
        Ladder { idle: &self.list.idle, hover: &self.list.hover, press: &self.list.press }
    }

    /// `class.tile` — the ladder the launcher's own tiles rest on.
    fn tiles(&self) -> Ladder<'_> {
        Ladder { idle: &self.tile.idle, hover: &self.tile.hover, press: &self.tile.press }
    }
}

/// How many rows of `pitch` fit in `h`, how far the list can be
/// scrolled, and which row is first — the row list's half of what
/// [`tile::layout`] does for tiles, over `list.*` rather than
/// `filetile.*`. The scroll is clamped here for the same reason it is
/// clamped there: the bounds are arithmetic only this function does.
fn rows(pitch: f32, gap: f32, h: f32, count: usize, scroll: &mut f32) -> tile::Scroll {
    let nvis = if pitch > 0.0 {
        (((h + gap) / pitch).floor() as usize).max(1)
    } else {
        1
    };
    let max_off = count.saturating_sub(nvis);
    *scroll = scroll.clamp(0.0, (max_off as f32 * pitch).max(0.0));
    let off = if pitch > 0.0 {
        ((*scroll / pitch).round() as usize).min(max_off)
    } else {
        0
    };
    tile::Scroll { total: count, nvis, off, max_off }
}

/// One row: the container on its rung, a chip carrying one mark, the
/// name, and the trailing count. The name is trimmed to whatever the
/// count leaves it, so a long category name never runs under its own
/// number.
#[allow(clippy::too_many_arguments)]
fn row(
    api: &HostApi,
    ctx: *mut c_void,
    look: &Look,
    rect: Rect,
    rung: &StateStyleC,
    mark: &str,
    label: &str,
    right: &str,
) {
    // `list.*` declares no corner of its own; a row is a container in
    // the same family as a tile, so it takes the tile's chamfer rather
    // than a number this file made up.
    let cut = look.tile.corner.min(rect.h / 2.0);
    tile::frame(api, ctx, rect.c(), cut, rung, look.tile.glow_scale);

    // The chip, and the mark centred in it. Never taller than the row.
    let chip_w = look.list.chip.min(rect.h);
    let chip = Rect::new(
        rect.x + look.list.pad_x,
        rect.y + (rect.h - chip_w) / 2.0,
        chip_w,
        chip_w,
    );
    let gpx = look.tile.glyph_px.min(chip.h).max(0.0);
    if !mark.is_empty() {
        tile::draw_text(api, ctx, gpx, chip.cx(), chip.cy() - gpx / 2.0, mark, rung.glyph, 0.0);
    }

    // The count, right-aligned on the row's inner edge. A number in a
    // column is `type.data`'s role, which the theme draws in the
    // monospace face with tabular figures — so a column of counts lines
    // up without this file arranging anything.
    let cpx = look.list.count_px;
    let csp = cpx * look.list.count_tracking;
    let count = tile::recase(look.list.count_case, right.to_string());
    let cx = rect.right() - look.list.pad_x;
    tile::text(
        api,
        ctx,
        look.list.count_font,
        cpx,
        cx,
        rect.cy() - cpx * look.list.count_leading / 2.0,
        &count,
        rung.text,
        csp,
        2,
    );

    // The name, in the caption role the launcher's tiles already use,
    // between the chip and whatever the count left.
    let px = look.tile.caption_px;
    let sp = px * look.tile.caption_tracking;
    let lx = chip.right() + look.list.chip_gap;
    let font = look.list.label_font;
    let room = cx
        - tile::measure(api, ctx, look.list.count_font, cpx, &count, csp)
        - look.list.status_gap
        - lx;
    let name = tile::recase(look.tile.caption_case, label.to_string());
    let name = tile::fit_name(api, ctx, font, px, &name, room.max(0.0), sp);
    tile::text(
        api,
        ctx,
        font,
        px,
        lx,
        rect.cy() - px * look.tile.caption_leading / 2.0,
        &name,
        rung.text,
        sp,
        0,
    );
}

/// What a box with nothing in it says. Nothing found is not an error —
/// a machine can honestly have no menu — so it says so in the caption
/// role rather than in a critical pill.
fn empty(api: &HostApi, ctx: *mut c_void, r: Rect, look: &Look, what: &str) {
    let px = look.tile.caption_px;
    let sp = px * look.tile.caption_tracking;
    let text = tile::recase(look.tile.caption_case, what.to_string());
    // The empty-state fraction says where in the box the line sits; the
    // role's own leading is what centres the line box on it rather than
    // hanging it below.
    tile::draw_text(
        api,
        ctx,
        px,
        r.cx(),
        r.y + r.h * look.list.empty_y - px * look.tile.caption_leading / 2.0,
        &text,
        look.tile.idle.text,
        sp,
    );
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Appcats::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Appcats) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Appcats> {
    unsafe { (instance as *mut Appcats).as_mut() }
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
        // Opening a group, leaving one, and starting an application are
        // all this widget's own; the host has nothing to do about any
        // of them.
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

/// Grows downwards: a taller panel is more groups, or more rows of
/// applications inside one — never bigger ones.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_ROWS
}

/// The header, as chrome: the panel's name on the left, and on the
/// right how many groups there are — or, inside a group, how many
/// applications it holds. Which of the two it is, the row under it says
/// in words.
extern "C" fn chrome_c(
    instance: *mut c_void,
    _ctx: *mut c_void,
    _host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    static TITLE: &[u8] = b"CATEGORIES";
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return 0;
    };
    let n = match this.open_index() {
        Some(i) => this.cats[i].apps.len(),
        None => this.cats.len(),
    };
    this.chrome_right = n.to_string().into_bytes();
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

#[cfg(test)]
mod row_tests {
    use super::rows;

    #[test]
    fn a_column_of_rows_scrolls_by_whole_rows_and_no_further() {
        // Ten rows of 20 in a box of 100: five fit, five are past the
        // bottom, and the furthest the list goes is the fifth row.
        let mut s = 0.0;
        let r = rows(20.0, 0.0, 100.0, 10, &mut s);
        assert_eq!((r.total, r.nvis, r.off, r.max_off), (10, 5, 0, 5));
        // A scroll inside a row rounds to the nearer row rather than
        // leaving half a row hanging off the top.
        let mut s = 51.0;
        assert_eq!(rows(20.0, 0.0, 100.0, 10, &mut s).off, 3);
        // Scrolled past the end: both the offset and the pixel figure
        // are pulled back, so the next wheel notch is not swallowed
        // undoing an overshoot.
        let mut s = 9999.0;
        assert_eq!(rows(20.0, 0.0, 100.0, 10, &mut s).off, 5);
        assert_eq!(s, 100.0);
        // A list that fits does not scroll at all.
        let mut s = 40.0;
        let r = rows(20.0, 0.0, 100.0, 3, &mut s);
        assert_eq!((r.nvis, r.off, r.max_off), (5, 0, 0));
        assert_eq!(s, 0.0);
        // The gap counts as space only BETWEEN rows: four rows of 20
        // with a gap of 5 fit a box of 95 (4*20 + 3*5), and a fifth
        // does not.
        assert_eq!(rows(25.0, 5.0, 95.0, 10, &mut 0.0).nvis, 4);
        assert_eq!(rows(25.0, 5.0, 94.0, 10, &mut 0.0).nvis, 3);
        // A theme that declares no row height at all draws one row of
        // nothing rather than dividing by zero.
        let r = rows(0.0, 0.0, 100.0, 10, &mut 0.0);
        assert_eq!((r.nvis, r.off), (1, 0));
    }
}
