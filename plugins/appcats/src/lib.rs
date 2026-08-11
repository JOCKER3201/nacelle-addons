//! CATEGORIES panel — the groups this machine's installed applications
//! fall into, as a list, and nothing else.
//!
//! It draws no applications. Clicking a group does not open it here: it
//! POINTS THE LAUNCHER GRID at it, and the grid next door redraws
//! showing that group alone. The chosen row stays visibly chosen, on
//! the state ladder's own `selected` rung, because a list that steers
//! something else and does not say which row is doing the steering is a
//! list of dead ends.
//!
//! The top row is ALL APPLICATIONS, with the whole menu's count, and it
//! is what a launcher nobody has clicked yet is on. Choosing it puts
//! the grid back on the whole menu, where the grid draws its
//! alphabetical index.
//!
//! # How the two widgets reach each other
//!
//! Through [`nacelle_widget_appgrid::selection`], and the long comment
//! at the head of that module is the one worth reading: the host has no
//! channel between widgets, this is a static cell that works only
//! because both widgets are linked into one binary, and the real fix is
//! an ABI in `libnacelle`. This file is one of that cell's two callers
//! — the writing one.
//!
//! It is the grid's sister, not its copy. The menu is found by the
//! grid's own XDG scanner, the groups are read by the grid's own
//! reading of the menu specification ([`cats`]), and the chamfer, the
//! glow, the caption role and the scrollbar are the grid's own. What
//! this widget adds is one question the grid does not ask — *which
//! group is this in* — and the row that answers it.
//!
//! Which group is chosen belongs to the SELECTION and not to the widget
//! instance: two of these panels on two boards are two views of one
//! launcher, so they show the same row chosen, because there is one
//! grid for them to be steering. How far each is scrolled is still the
//! instance's own.
//!
//! Every colour, length, duration and word comes from the theme through
//! ABI 5/6 tokens. Nothing here knows what a colour is: a missing token
//! degrades through the raw answers the ABI itself gives (grey ink,
//! zero lengths), never through a number that used to be the design.

use nacelle::runtime::{
    ActionC, ChromeC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_NONE,
};
use nacelle_widget_appgrid::cats::{self, Category};
use nacelle_widget_appgrid::desktop::{self, AppEntry};
use nacelle_widget_appgrid::selection::{self, Selection};
use nacelle_widget_appgrid::tile::{self, Rect, TileLook, TileTheme};
use std::ffi::c_void;
use std::time::Instant;

/// How often the menu is looked at again — the grid's own rate, for the
/// same reason: the scan only runs when the directories' modification
/// times have MOVED, so this is a handful of `stat` calls.
const RESCAN_SECS: u64 = 5;

/// The top row's name. English in the code, as every string in this
/// tree is: what a user reads is the theme's and the locale's business,
/// not this file's.
const ALL_LABEL: &str = "ALL APPLICATIONS";

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
    selected: StateStyleC,
    selected_hover: StateStyleC,
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
            selected: raw_state,
            selected_hover: raw_state,
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
            selected: tile::rung(api, ctx, t.item_class, tile::STATE_SELECTED),
            selected_hover: tile::rung(api, ctx, t.item_class, tile::STATE_SELECTED_HOVER),
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

/// What the pointer can be over. The press flash, the selection test
/// and the click path all speak in these rather than in row numbers, so
/// that the top row cannot be confused with the first group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// The top row: the whole menu, whatever it is grouped under.
    All,
    /// A group, by its place in [`Appcats::cats`].
    Cat(usize),
}

pub struct Appcats {
    /// The installed applications, sorted by display name — the grid's
    /// own list, scanned by the grid's own scanner. Kept for the count
    /// on the ALL APPLICATIONS row, which has to be the menu's own
    /// figure and not the sum of the groups: an entry in both
    /// `AudioVideo` and `Audio` is counted by both of those and once by
    /// this one.
    entries: Vec<AppEntry>,
    /// The groups those entries fall into, alphabetically.
    cats: Vec<Category>,
    /// What the launcher is pointed at, as of this frame. Read from the
    /// shared cell rather than held here: another panel of this widget
    /// may have been the one that set it.
    sel: Selection,
    /// Scroll offset in pixels; whole rows.
    scroll: f32,
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
            // Whatever the launcher is already on. NOT reset to ALL:
            // a second categories panel opened later must show the
            // choice already made, not undo it.
            sel: selection::get(),
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
        self.cats = cats::group(&self.entries);
        eprintln!(
            "appcats: menu changed \u{2014} {} categories over {} applications",
            self.cats.len(),
            self.entries.len()
        );
        // The chosen group can have stopped existing — its last
        // application uninstalled while it was being looked at. Being
        // put back on the whole menu is the honest answer to that; a
        // row chosen in a list that no longer holds it would be a
        // selection pointing at nothing, and an empty grid with no way
        // to say why.
        if let Some(name) = selection::get().name() {
            if !self.cats.iter().any(|c| c.name == name) {
                selection::set(Selection::All);
            }
        }
    }

    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).max(0.0);
    }

    /// A click on a row points the launcher grid at what that row says,
    /// and nothing else — the grid reads the same cell on its next
    /// frame. There is no action for the host to take: `ActionC` has no
    /// code that means "another widget should now show something else",
    /// and inventing one is the ABI change this widget is working
    /// around rather than pre-empting (see
    /// [`nacelle_widget_appgrid::selection`]).
    pub fn click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|&(_, h)| h)
        else {
            return;
        };
        self.pressed = Some((hit, Instant::now()));
        let what = match hit {
            Hit::All => Selection::All,
            Hit::Cat(i) => {
                let Some(c) = self.cats.get(i) else { return };
                Selection::Named(c.name.to_string())
            }
        };
        // Kept here as well as written to the cell, so that the rest of
        // THIS frame's hit list and the next draw agree without waiting
        // for a read back.
        self.sel = what.clone();
        selection::set(what);
    }

    /// Whether a row is the chosen one. By NAME and not by row number,
    /// for the reason the selection holds a name: a rescan rebuilds the
    /// groups, and installing a game where there were none moves every
    /// group after it by one.
    fn chosen(&self, hit: Hit) -> bool {
        match (hit, self.sel.name()) {
            (Hit::All, None) => true,
            (Hit::Cat(i), Some(name)) => {
                self.cats.get(i).map(|c| c.name == name).unwrap_or(false)
            }
            _ => false,
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
        // The chosen row can have been chosen in the other panel.
        self.sel = selection::get();
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

        if self.entries.is_empty() {
            empty(api, ctx, r, &look, "no applications");
            return;
        }

        // ALL APPLICATIONS at the top, then the groups. One list and
        // one scroll: the top row is a row like any other, so it
        // scrolls out of the way like any other rather than being
        // pinned by a rule this file made up.
        let pitch = look.list.row_h + look.list.row_gap;
        let count = self.cats.len() + 1;
        let s = rows(pitch, look.list.row_gap, r.h, count, &mut self.scroll);

        for n in s.off..(s.off + s.nvis).min(count) {
            let rect = Rect::new(r.x, r.y + (n - s.off) as f32 * pitch, r.w, look.list.row_h);
            let (hit, label, right) = match n {
                // The whole menu's own figure: how many applications
                // are installed, which is what the grid then shows.
                0 => (Hit::All, ALL_LABEL, self.entries.len()),
                _ => {
                    let c = &self.cats[n - 1];
                    (Hit::Cat(n - 1), c.name, c.apps.len())
                }
            };
            let rung = pointer.rung(look.rows(), rect, hit, self.chosen(hit));
            // The chip's mark is the label's own initial, the same rule
            // every row follows and the same one a tile follows for its
            // application. ALL APPLICATIONS gets an `A` by that rule and
            // is not given a symbol of its own: a mark this file
            // invented would be a glyph the shipped faces may not have,
            // and the row's name is beside it either way.
            row(api, ctx, &look, rect, rung, &tile::initial(label), label, &right.to_string());
            self.hits.push((rect, hit));
        }
        tile::scrollbar(api, ctx, &look.tile, r, s);
    }
}

impl Default for Appcats {
    fn default() -> Self {
        Appcats::new()
    }
}

/// Everything the state ladder needs to know about this frame's
/// pointer, so that picking a rung is one call rather than the same
/// five-armed `if` at every drawing site.
#[derive(Clone, Copy)]
struct Pointer {
    x: f32,
    y: f32,
    flashing: Option<Hit>,
}

impl Pointer {
    /// Which rung of a ladder the thing in `r` is resting on this
    /// frame.
    ///
    /// The order is the ladder's own reading of what is momentary and
    /// what is persistent: `press` first, because a click's flash is
    /// the answer to the click and lasts only as long as
    /// `motion.press.duration_ms` says; then the two `selected` rungs,
    /// which say this row is the chosen one whether or not the pointer
    /// is on it; then `hover`; then rest. The theme's own
    /// `selected_hover` exists precisely because "chosen AND pointed
    /// at" is a state a list produces, so it is asked for rather than
    /// approximated by one of the other two.
    fn rung<'a>(&self, l: Ladder<'a>, r: Rect, what: Hit, chosen: bool) -> &'a StateStyleC {
        let under = r.contains(self.x, self.y);
        match (self.flashing == Some(what), chosen, under) {
            (true, _, _) => l.press,
            (_, true, true) => l.selected_hover,
            (_, true, false) => l.selected,
            (_, false, true) => l.hover,
            (_, false, false) => l.idle,
        }
    }
}

/// The rungs of one class's state ladder that this widget uses.
#[derive(Clone, Copy)]
struct Ladder<'a> {
    idle: &'a StateStyleC,
    hover: &'a StateStyleC,
    press: &'a StateStyleC,
    selected: &'a StateStyleC,
    selected_hover: &'a StateStyleC,
}

impl Look {
    /// `class.list.item` — the ladder a row rests on.
    fn rows(&self) -> Ladder<'_> {
        Ladder {
            idle: &self.list.idle,
            hover: &self.list.hover,
            press: &self.list.press,
            selected: &self.list.selected,
            selected_hover: &self.list.selected_hover,
        }
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
        // Choosing a group is a fact about the launcher, written where
        // the launcher reads it; the host has nothing to do about it.
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

/// Grows downwards: a taller panel is more groups on screen, never
/// bigger ones.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_ROWS
}

/// The header, as chrome: the panel's name on the left, and on the
/// right how many groups this machine has.
///
/// The groups, not the rows: ALL APPLICATIONS is a way to the whole
/// menu and not a group the machine has, and it carries the menu's own
/// count on its own row anyway.
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
    this.chrome_right = this.cats.len().to_string().into_bytes();
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

#[cfg(test)]
mod list_tests {
    use super::*;

    fn entry(name: &str, categories: &[&str]) -> AppEntry {
        AppEntry {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.to_string(),
            exec: "/bin/true".to_string(),
            terminal: false,
            icon: String::new(),
            categories: categories.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A widget with a menu but no host: enough to exercise the row
    /// list, the selection and the chosen-row test, none of which touch
    /// the theme or draw anything.
    fn panel(entries: Vec<AppEntry>) -> Appcats {
        let cats = cats::group(&entries);
        Appcats {
            entries,
            cats,
            sel: Selection::All,
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: 0,
            theme: None,
            wheel_px: 0.0,
            chrome_right: Vec::new(),
        }
    }

    #[test]
    fn the_top_row_is_the_whole_menu_and_is_chosen_until_something_else_is() {
        let p = panel(vec![
            entry("Player", &["AudioVideo", "Audio"]),
            entry("Editor", &["Utility"]),
            entry("Toolkit", &["Qt"]),
        ]);
        // The list is the groups plus one, and the one is on top.
        assert_eq!(p.cats.iter().map(|c| c.name).collect::<Vec<_>>(), [
            "Audio",
            "AudioVideo",
            "Other",
            "Utility"
        ]);
        // ALL APPLICATIONS counts the MENU and not the memberships:
        // three applications, four group memberships, because the
        // player is in both AudioVideo and Audio. The top row must show
        // the three — it is what the grid then draws — and never the
        // sum of the rows under it.
        assert_eq!(p.entries.len(), 3);
        assert_eq!(p.cats.iter().map(|c| c.apps.len()).sum::<usize>(), 4);
        // Nothing clicked: the top row is the chosen one and no group
        // is.
        assert!(p.chosen(Hit::All));
        for i in 0..p.cats.len() {
            assert!(!p.chosen(Hit::Cat(i)), "{} is not chosen", p.cats[i].name);
        }
    }

    #[test]
    fn choosing_a_row_points_the_grid_and_moves_the_mark_with_it() {
        let mut p = panel(vec![
            entry("Player", &["AudioVideo", "Audio"]),
            entry("Editor", &["Utility"]),
        ]);
        let at = |p: &Appcats, name: &str| {
            Hit::Cat(p.cats.iter().position(|c| c.name == name).unwrap())
        };
        // Clicking a group is written to the cell the grid reads, and
        // marks that row and only that row.
        let utility = at(&p, "Utility");
        p.sel = Selection::Named("Utility".to_string());
        selection::set(p.sel.clone());
        assert_eq!(selection::get(), Selection::Named("Utility".to_string()));
        assert!(p.chosen(utility));
        assert!(!p.chosen(Hit::All));
        assert!(!p.chosen(at(&p, "Audio")));

        // Switching moves the mark rather than adding a second one.
        let audio = at(&p, "Audio");
        p.sel = Selection::Named("Audio".to_string());
        assert!(p.chosen(audio));
        assert!(!p.chosen(utility));

        // And the top row takes it back.
        p.sel = Selection::All;
        assert!(p.chosen(Hit::All));
        assert!(!p.chosen(audio));

        // A chosen group the menu no longer has marks nothing at all —
        // which is the state `follow` then puts back on ALL.
        p.sel = Selection::Named("Science".to_string());
        assert!(!p.chosen(Hit::All));
        for i in 0..p.cats.len() {
            assert!(!p.chosen(Hit::Cat(i)));
        }
        assert!(!p.cats.iter().any(|c| c.name == "Science"));

        // Put the shared cell back: it is process-wide, and a test that
        // left it set would be steering every other test's grid.
        selection::set(Selection::All);
    }

    #[test]
    fn an_empty_group_is_a_row_that_can_still_be_chosen() {
        // A group with nothing in it is never OFFERED — `group` only
        // returns groups that hold something — so the list cannot make
        // one. What it can do is be pointed at one by a rescan, and the
        // grid answers that with an empty page rather than a stale one.
        let p = panel(vec![entry("Editor", &["Utility"])]);
        assert!(p.cats.iter().all(|c| !c.apps.is_empty()));
        assert!(!p.cats.iter().any(|c| c.name == "Game"));
        let view: Vec<usize> = p
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| cats::holds("Game", e))
            .map(|(i, _)| i)
            .collect();
        assert!(view.is_empty(), "the grid draws nothing for a group nothing is in");
        // And a machine with no menu at all has no groups, so the panel
        // has only its top row to draw — which the draw path answers
        // with the empty state instead.
        let none = panel(Vec::new());
        assert!(none.cats.is_empty());
        assert!(none.entries.is_empty());
    }
}
