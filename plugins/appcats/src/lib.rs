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
//! Through the HOST. A click publishes the chosen group's name on
//! `nacelle::channel` under [`nacelle_launcher_core::selection::TOPIC`],
//! and the grid next door reads it on its next frame — see the head of
//! [`nacelle_launcher_core::selection`] for why the value cannot live in
//! either widget or in the crate they share: two `.so` files carry two
//! copies of that crate, and only the host has one of anything. This
//! file is the WRITING end of that channel.
//!
//! It is the grid's sister, not its copy. The menu is found by the ONE
//! XDG scanner, the groups are read by the ONE reading of the menu
//! specification ([`cats`]), and the chamfer, the glow, the caption
//! role and the scrollbar are the ONE tile grid — all of it
//! `nacelle-launcher-core`, the half both widgets are built out of and
//! neither owns. What this widget adds is one question the grid does
//! not ask — *which group is this in* — and the row that answers it.
//!
//! Which group is chosen belongs to the SELECTION and not to the widget
//! instance: two of these panels on two boards are two views of one
//! launcher, so they show the same row chosen, because there is one
//! grid for them to be steering. How far each is scrolled is still the
//! instance's own.
//!
//! Every colour, length, duration and word comes from the theme through
//! ABI 5/6 tokens. Nothing here knows what a colour is: a token nobody
//! can answer degrades to no ink and no length — nothing drawn — never
//! to a number that used to be the design.

use nacelle::runtime::{
    ActionC, ChromeC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_CAPTURE,
    ACTION_NONE, CORNER_SQUARE, DRAG_BEGIN, DRAG_END, DRAG_MOVE,
};
use nacelle::ui::Case;
use nacelle::widget::factory::BuiltinWidget;
use nacelle_launcher_core::cats::{self, Category};
use nacelle_launcher_core::desktop::{self, AppEntry};
use nacelle_launcher_core::selection::{Selection, Watch};
use nacelle_launcher_core::tile::{self, EmptyLook, EmptyTheme, Rect, TileLook, TileTheme};
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
/// the state matrix describes in exactly those words. A row used to
/// borrow the tile grid's notch for want of one of its own; the list
/// group now declares `list.wheel_px`, so nothing here borrows.
struct ListTheme {
    epoch: u32,
    row_h: u32,      // list.row_h
    row_gap: u32,    // list.gap
    pad_x: u32,      // list.pad_x
    chip: u32,       // list.glyph — the leading coloured chip of a row
    chip_gap: u32,   // list.glyph_gap
    status_gap: u32, // list.status_gap — the gap before a row's trailing status
    wheel: u32,      // list.wheel_px — a row list scrolls by its own notch, not a tile grid's
    /// The plate behind a row: `list.corner` is its radius and
    /// `list.corner_style` the cut. A row used to borrow the file
    /// browser's `filetile.corner` for want of one of its own, which is
    /// exactly what the master's `list.corner` comment says it was for.
    corner: u32,
    /// The cut itself, decoded from `list.corner_style`'s WORD beside
    /// the ids: a word crossing the boundary is a string copy, and a
    /// string copy per frame per row is not a thing to do in a draw.
    corner_cut: u32,
    // type.<list.status_role>.* — the trailing count. The master binds
    // it to `caption`; this file used to spell `type.data.*` out, which
    // is a second binding nobody can edit.
    status_size: u32,
    status_min: u32,
    status_tracking: u32,
    status_leading: u32,
    status_case: u32,
    // type.<list.label_role>.* — the row's name. Bound to `body`, a
    // step ABOVE the caption size this file drew it at, so the list is
    // the size the master says and not one grade smaller.
    label_size: u32,
    label_min: u32,
    label_tracking: u32,
    label_leading: u32,
    label_case: u32,
    /// The font slots those two roles' `face` names, resolved WITH the
    /// ids because a face is a word and reading words is init-time
    /// work. A role asking for the monospace face means it: a column of
    /// counts lines up because the figures are tabular, which is a
    /// property of the face and not of this file.
    status_font: u32,
    label_font: u32,
    /// A row's row in the class x state matrix.
    item_class: u32,
}

impl ListTheme {
    fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> ListTheme {
        // The row's two type bindings, followed to the roles they name.
        let label = tile::enum_word(api, ctx, tile::token(api, "list.label_role"));
        let status = tile::enum_word(api, ctx, tile::token(api, "list.status_role"));
        ListTheme {
            epoch,
            row_h: tile::token(api, "list.row_h"),
            row_gap: tile::token(api, "list.gap"),
            pad_x: tile::token(api, "list.pad_x"),
            chip: tile::token(api, "list.glyph"),
            chip_gap: tile::token(api, "list.glyph_gap"),
            status_gap: tile::token(api, "list.status_gap"),
            wheel: tile::token(api, "list.wheel_px"),
            corner: tile::token(api, "list.corner"),
            corner_cut: tile::corner_style(
                &tile::enum_word(api, ctx, tile::token(api, "list.corner_style")),
            ),
            status_size: tile::role_id(api, &status, "size"),
            status_min: tile::role_id(api, &status, "min_px"),
            status_tracking: tile::role_id(api, &status, "tracking"),
            status_leading: tile::role_id(api, &status, "leading"),
            status_case: tile::role_id(api, &status, "case"),
            label_size: tile::role_id(api, &label, "size"),
            label_min: tile::role_id(api, &label, "min_px"),
            label_tracking: tile::role_id(api, &label, "tracking"),
            label_leading: tile::role_id(api, &label, "leading"),
            label_case: tile::role_id(api, &label, "case"),
            status_font: tile::face_slot(api, ctx, tile::role_id(api, &status, "face")),
            label_font: tile::face_slot(api, ctx, tile::role_id(api, &label, "face")),
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
    wheel_px: f32,
    corner: tile::Corner,
    status_px: f32,
    status_tracking: f32,
    status_leading: f32,
    status_case: Case,
    label_px: f32,
    label_tracking: f32,
    label_leading: f32,
    label_case: Case,
    status_font: u32,
    label_font: u32,
}

impl ListLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// Zero lengths, the matrix's own raw ink — an old host shows the
    /// same undesigned raw as an empty theme.
    fn raw() -> ListLook {
        let raw_state = StateStyleC {
            fill: tile::NO_COLOR,
            edge: tile::NO_COLOR,
            text: tile::NO_COLOR,
            glyph: tile::NO_COLOR,
            edge_width: 0.0,
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
            wheel_px: 0.0,
            corner: tile::Corner { style: CORNER_SQUARE, radius: 0.0 },
            status_px: 0.0,
            status_tracking: 0.0,
            status_leading: 1.0,
            status_case: Case::None,
            label_px: 0.0,
            label_tracking: 0.0,
            label_leading: 1.0,
            label_case: Case::None,
            status_font: tile::FONT_UI,
            label_font: tile::FONT_UI,
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
            wheel_px: px(t.wheel),
            corner: tile::Corner { style: t.corner_cut, radius: px(t.corner) },
            status_px: px(t.status_size).max(px(t.status_min)),
            status_tracking: px(t.status_tracking),
            status_leading: px(t.status_leading).max(1.0),
            status_case: Case::from_word(&tile::enum_word(api, ctx, t.status_case)),
            label_px: px(t.label_size).max(px(t.label_min)),
            label_tracking: px(t.label_tracking),
            label_leading: px(t.label_leading).max(1.0),
            label_case: Case::from_word(&tile::enum_word(api, ctx, t.label_case)),
            status_font: t.status_font,
            label_font: t.label_font,
        }
    }
}

/// Both halves of this widget's look, read once per frame.
struct Look {
    tile: TileLook,
    list: ListLook,
    /// The line this list draws INSTEAD of its rows. Its own look,
    /// because "the panel has nothing to show" is its own kind of
    /// element and the master answers it once, in `emptystate.role`.
    empty: EmptyLook,
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
    /// What the launcher is pointed at, as of this frame. A view of the
    /// host's board rather than a fact of this instance: another panel
    /// of this widget may have been the one that set it.
    sel: Watch,
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
    theme: Option<(TileTheme, ListTheme, EmptyTheme)>,
    /// `list.wheel_px`, cached at draw because a wheel event arrives
    /// with no drawing context to ask the theme through.
    wheel_px: f32,
    /// The bar the last frame drew, or none when there was nothing to
    /// scroll — the rectangles AND the bottom the offset may reach,
    /// which is everything a press arriving between two frames needs to
    /// answer for itself.
    bar: Option<tile::BarGeom>,
    /// The thumb under the hand, while there is one.
    grab: tile::ThumbGrab,
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
            // A view that has not looked yet; the first frame's poll
            // adopts whatever the launcher is already on. NOT a reset to
            // ALL: a second categories panel opened later must show the
            // choice already made, not undo it.
            sel: Watch::new(),
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: desktop::stamp(),
            theme: None,
            wheel_px: 0.0,
            bar: None,
            grab: tile::ThumbGrab::default(),
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
        self.sel.poll();
        let gone = self
            .sel
            .get()
            .name()
            .is_some_and(|name| !self.cats.iter().any(|c| c.name == name));
        if gone {
            self.sel.set(Selection::All);
        }
    }

    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).max(0.0);
    }

    /// A pointer press. `true` when the list took the gesture — the host
    /// then captures the pointer and no click is delivered when it is
    /// let go.
    ///
    /// Only the bar takes a press: everything else here is a row, and a
    /// row steers the grid on the RELEASE, by `click`, exactly as it
    /// always has been.
    pub fn press(&mut self, x: f32, y: f32) -> bool {
        let Some(bar) = self.bar else { return false };
        if !bar.track.contains(x, y) {
            return false;
        }
        if self.grab.press(y, &bar) {
            return true;
        }
        // Beside the thumb: one page toward the click, where a page is
        // the content box the bar stands in. The press is still taken —
        // the bar lies ON TOP of the rows, and letting it through would
        // point the grid at a group the hand never aimed at.
        let page = bar.track.h;
        self.scroll = if y >= bar.thumb.y + bar.thumb.h {
            (self.scroll + page).min(bar.max_px)
        } else {
            (self.scroll - page).max(0.0)
        };
        true
    }

    /// The pointer moved while it held the thumb. Only the y matters:
    /// the thumb goes where the hand is, and a hand that wandered off
    /// the bar sideways is still holding it.
    ///
    /// The offset lands on a whole row on the next frame, where [`rows`]
    /// rounds it — the same snapping the wheel gets, for the same
    /// reason.
    pub fn drag_to(&mut self, y: f32) {
        let Some(bar) = self.bar else { return };
        if let Some(px) = self.grab.drag_to(y, &bar) {
            self.scroll = px;
        }
    }

    /// The pointer let go.
    pub fn release(&mut self) {
        self.grab.release();
    }

    /// A click on a row points the launcher grid at what that row says,
    /// and nothing else — the grid reads the same topic on its next
    /// frame. There is still no action for the host to take: `ActionC`
    /// has no code that means "another widget should now show something
    /// else", and it needs none, because the channel says it without
    /// naming the widget that listens.
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
        // Published and adopted in one call, so the rest of THIS
        // frame's hit list and the next draw agree without waiting for
        // a read back — see [`Watch::set`].
        self.sel.set(what);
    }

    /// Whether a row is the chosen one. By NAME and not by row number,
    /// for the reason the selection holds a name: a rescan rebuilds the
    /// groups, and installing a game where there were none moves every
    /// group after it by one.
    fn chosen(&self, hit: Hit) -> bool {
        match (hit, self.sel.get().name()) {
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
            return Look {
                tile: TileLook::raw(),
                list: ListLook::raw(),
                empty: EmptyLook::raw(),
            };
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|(t, _, _)| t.epoch) != Some(epoch) {
            self.theme = Some((
                TileTheme::resolve(api, ctx, epoch),
                ListTheme::resolve(api, ctx, epoch),
                EmptyTheme::resolve(api, ctx, epoch),
            ));
        }
        match &self.theme {
            Some((t, l, e)) => {
                debug_assert_eq!(t.epoch, l.epoch);
                debug_assert_eq!(t.epoch, e.epoch);
                Look {
                    tile: TileLook::read(api, ctx, t),
                    list: ListLook::read(api, ctx, l),
                    empty: EmptyLook::read(api, ctx, e),
                }
            }
            None => Look {
                tile: TileLook::raw(),
                list: ListLook::raw(),
                empty: EmptyLook::raw(),
            },
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        // Cleared here rather than only on the path that draws a bar: a
        // frame with nothing to scroll must leave no rectangle behind
        // for the next press to take hold of.
        self.bar = None;
        self.follow();
        // The chosen row can have been chosen in the other panel, or in
        // another copy of this one. Cheap on the frames where it was
        // not: a sequence number, not a copy of the value.
        self.sel.poll();
        let look = self.look(api, ctx);
        self.wheel_px = look.list.wheel_px;

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
        // And the same numbers again, kept for the hand: a press arrives
        // between two frames with no geometry of its own, and `bar_geom`
        // is the function the drawing above went through, so what is
        // grabbed is what was seen.
        self.bar = tile::bar_geom(&look.tile, r, s);
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
///
/// The answer carries that bottom IN PIXELS as well as in rows, handed
/// back rather than recomputed by the caller: a dragged thumb divides by
/// it, and two spellings of one clamp are a thumb that can be pulled
/// past the end of its own list.
fn rows(pitch: f32, gap: f32, h: f32, count: usize, scroll: &mut f32) -> tile::Scroll {
    let nvis = if pitch > 0.0 {
        (((h + gap) / pitch).floor() as usize).max(1)
    } else {
        1
    };
    let max_off = count.saturating_sub(nvis);
    let max_px = (max_off as f32 * pitch).max(0.0);
    *scroll = scroll.clamp(0.0, max_px);
    let off = if pitch > 0.0 {
        ((*scroll / pitch).round() as usize).min(max_off)
    } else {
        0
    };
    tile::Scroll { total: count, nvis, off, px: off as f32 * pitch, max_px }
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
    // The plate behind the row, on the row family's OWN shape:
    // `list.corner` for the radius and `list.corner_style` for the cut.
    // It borrowed the file browser's tile corner until the two keys
    // existed, which is what `list.corner`'s own comment records.
    let corner = tile::Corner {
        radius: look.list.corner.radius.min(rect.h / 2.0),
        ..look.list.corner
    };
    tile::frame(api, ctx, rect.c(), corner, rung, look.tile.glow_scale);

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

    // The count, right-aligned on the row's inner edge, in the role
    // `list.status_role` names — an aside beside the name, and the
    // theme's word for which type an aside is set in. Whether its
    // figures line up in a column is the role's face, not this file's
    // arrangement.
    let cpx = look.list.status_px;
    let csp = cpx * look.list.status_tracking;
    let count = tile::recase(look.list.status_case, right);
    let cx = rect.right() - look.list.pad_x;
    tile::text(
        api,
        ctx,
        look.list.status_font,
        cpx,
        cx,
        rect.cy() - cpx * look.list.status_leading / 2.0,
        &count,
        rung.text,
        csp,
        2,
    );

    // The name, in the role `list.label_role` names, between the chip
    // and whatever the count left. It was drawn at the launcher tile's
    // caption size — one grade under what the master asks a row of
    // prose to be.
    let px = look.list.label_px;
    let sp = px * look.list.label_tracking;
    let lx = chip.right() + look.list.chip_gap;
    let font = look.list.label_font;
    let room = cx
        - tile::measure(api, ctx, look.list.status_font, cpx, &count, csp)
        - look.list.status_gap
        - lx;
    let name = tile::recase(look.list.label_case, label);
    let name = tile::fit_name(api, ctx, font, px, &name, room.max(0.0), sp);
    tile::text(
        api,
        ctx,
        font,
        px,
        lx,
        rect.cy() - px * look.list.label_leading / 2.0,
        &name,
        rung.text,
        sp,
        0,
    );
}

/// What a box with nothing in it says. Nothing found is not an error —
/// a machine can honestly have no menu — so it says so in a line rather
/// than in a critical pill.
fn empty(api: &HostApi, ctx: *mut c_void, r: Rect, look: &Look, what: &str) {
    // In `emptystate.role`, which is the master's answer for every panel
    // that has nothing to show — not in this list's ROW role.
    //
    // The row argument stood here and was not wrong about this panel: a
    // list's empty line does sit where a row would. It was answering a
    // question the theme had already answered, and the cost was visible
    // on one board — this sentence at 13.3 px beside the launcher grid
    // saying the same words at 9.6 px, while the search panel next to
    // both said it at 17.6 px.
    tile::empty_line(
        api,
        ctx,
        &look.empty,
        r,
        what,
        // The list's own resting ink, so the line reads as part of the
        // surface the rows would have covered.
        Some(look.tile.idle.text),
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
        // list.wheel_px, as the last draw cached it — a wheel event
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

/// The pointer's whole gesture — the host's single capture path, and
/// what this list's scroll thumb is dragged by.
///
/// A `Begin` anywhere but on the bar is DECLINED (`ACTION_NONE`), which
/// leaves the press on the ordinary click path: that is how a row still
/// steers the grid by releasing on it. A `Begin` on the bar answers
/// `ACTION_CAPTURE` — the gesture is the widget's — and the host then
/// routes every motion here and no click at the end.
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

/// Filled and consumes nothing, on purpose: this list is walked with
/// the pointer and has no keyboard behaviour to take a key away for.
/// The arrows are the focus chain's until this panel grows a keyboard
/// cursor of its own, and answering 0 is what leaves them there.
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

/// Filled and does nothing, on purpose. The press rung this entry
/// carries is one this panel already draws from its own clock — a row
/// marks itself for `motion.press.duration_ms` from the click — so
/// taking the press here as well would be a second source of one state.
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
/// `appcats.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `appcats.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "appcats",
    meta: include_str!("../appcats.meta"),
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
mod row_tests {
    use super::rows;

    #[test]
    fn a_column_of_rows_scrolls_by_whole_rows_and_no_further() {
        // Ten rows of 20 in a box of 100: five fit, five are past the
        // bottom, and the furthest the list goes is the fifth row.
        let mut s = 0.0;
        let r = rows(20.0, 0.0, 100.0, 10, &mut s);
        assert_eq!((r.total, r.nvis, r.off), (10, 5, 0));
        // The pixel bottom is the row bottom said in the units the
        // offset is kept in — five rows of twenty — and the position is
        // said in the same units, which for a column of equal rows is
        // the row index times the pitch.
        assert_eq!((r.px, r.max_px), (0.0, 100.0));
        // A scroll inside a row rounds to the nearer row rather than
        // leaving half a row hanging off the top, and the pixel figure
        // beside it names that same row.
        let mut s = 51.0;
        let r = rows(20.0, 0.0, 100.0, 10, &mut s);
        assert_eq!((r.off, r.px), (3, 60.0));
        // Scrolled past the end: both the offset and the pixel figure
        // are pulled back, so the next wheel notch is not swallowed
        // undoing an overshoot.
        let mut s = 9999.0;
        let r = rows(20.0, 0.0, 100.0, 10, &mut s);
        assert_eq!((r.off, r.px), (5, 100.0));
        assert_eq!(s, 100.0);
        // A list that fits does not scroll at all.
        let mut s = 40.0;
        let r = rows(20.0, 0.0, 100.0, 3, &mut s);
        assert_eq!((r.nvis, r.off), (5, 0));
        assert_eq!((s, r.px, r.max_px), (0.0, 0.0, 0.0));
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
            sel: Watch::new(),
            scroll: 0.0,
            hits: Vec::new(),
            pressed: None,
            last_look: Instant::now(),
            stamp: 0,
            theme: None,
            wheel_px: 0.0,
            bar: None,
            grab: tile::ThumbGrab::default(),
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

    /// The ONE test in this crate that writes the host's board. The
    /// board is process-wide, so a second writer would race this one
    /// under the default harness; every other test here builds a panel
    /// whose [`Watch`] has not polled, which is why none of them can see
    /// what this one publishes.
    #[test]
    fn choosing_a_row_points_the_grid_and_moves_the_mark_with_it() {
        let mut p = panel(vec![
            entry("Player", &["AudioVideo", "Audio"]),
            entry("Editor", &["Utility"]),
        ]);
        let at = |p: &Appcats, name: &str| {
            Hit::Cat(p.cats.iter().position(|c| c.name == name).unwrap())
        };
        // Choosing a group publishes it for the grid to read, and marks
        // that row and only that row. A second view of the board — a
        // grid in another `.so`, as far as this crate is concerned —
        // hears exactly what was chosen.
        let mut grid = Watch::new();
        let utility = at(&p, "Utility");
        p.sel.set(Selection::Named("Utility".to_string()));
        assert!(grid.poll(), "the grid hears the click");
        assert_eq!(grid.get().name(), Some("Utility"));
        assert!(p.chosen(utility));
        assert!(!p.chosen(Hit::All));
        assert!(!p.chosen(at(&p, "Audio")));

        // Switching moves the mark rather than adding a second one.
        let audio = at(&p, "Audio");
        p.sel.set(Selection::Named("Audio".to_string()));
        assert!(grid.poll());
        assert_eq!(grid.get().name(), Some("Audio"));
        assert!(p.chosen(audio));
        assert!(!p.chosen(utility));

        // And the top row takes it back.
        p.sel.set(Selection::All);
        assert!(grid.poll());
        assert!(grid.get().is_all());
        assert!(p.chosen(Hit::All));
        assert!(!p.chosen(audio));

        // A chosen group the menu no longer has marks nothing at all —
        // which is the state `follow` then puts back on ALL.
        p.sel.set(Selection::Named("Science".to_string()));
        assert!(!p.chosen(Hit::All));
        for i in 0..p.cats.len() {
            assert!(!p.chosen(Hit::Cat(i)));
        }
        assert!(!p.cats.iter().any(|c| c.name == "Science"));

        // Put the board back: it is process-wide, and a test that left
        // it set would be steering every other test's grid.
        p.sel.set(Selection::All);
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

    /// A panel carrying the bar a frame would have drawn: a 100-px track
    /// with a 20-px thumb at its top, over 300 px of rows below the
    /// fold. Built by hand because the bar is a fact of the last DRAW,
    /// and no test here has a host to draw through.
    fn panel_with_a_bar() -> Appcats {
        let mut p = panel(Vec::new());
        p.bar = Some(tile::BarGeom {
            track: Rect::new(90.0, 0.0, 6.0, 100.0),
            thumb: Rect::new(90.0, 0.0, 6.0, 20.0),
            max_px: 300.0,
        });
        p
    }

    /// A value no entry of this widget could ever write, so "left alone"
    /// is something a test can see.
    fn untouched() -> ActionC {
        ActionC { kind: u32::MAX, index: 0, lines: 0, data: std::ptr::null(), data_len: 0 }
    }

    /// The drag entry, driven through the TABLE: a Begin on the thumb
    /// asks the host for the pointer, a Begin beside the bar does not.
    /// The capture is the whole of it — without it the host delivers the
    /// press as an ordinary click, points the grid at whatever row is
    /// under the bar, and no motion ever reaches this widget.
    #[test]
    fn a_press_on_the_thumb_asks_for_the_capture_and_one_beside_it_does_not() {
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut p = panel_with_a_bar();
        let inst = &mut p as *mut Appcats as *mut c_void;

        let mut a = untouched();
        (API.drag)(inst, DRAG_BEGIN, 92.0, 10.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_CAPTURE);
        (API.drag)(inst, DRAG_END, 92.0, 10.0, r, 100.0, 100.0, &mut a);

        // Beside the bar, over the rows: not ours, so the press stays on
        // the click path that steers the grid.
        let mut b = untouched();
        (API.drag)(inst, DRAG_BEGIN, 10.0, 10.0, r, 100.0, 100.0, &mut b);
        assert_eq!(b.kind, ACTION_NONE);

        // A phase from a newer host is no gesture, and a null instance
        // is a decline rather than a crash.
        let mut c = untouched();
        (API.drag)(inst, 999, 92.0, 10.0, r, 100.0, 100.0, &mut c);
        assert_eq!(c.kind, ACTION_NONE);
        let mut d = untouched();
        (API.drag)(std::ptr::null_mut(), DRAG_BEGIN, 92.0, 10.0, r, 100.0, 100.0, &mut d);
        assert_eq!(d.kind, ACTION_NONE);
    }

    /// And the motion that follows the capture MOVES the list: the thumb
    /// goes where the hand is, absolutely. Half the 80 px of travel over
    /// 300 px below the fold is 150 px of offset.
    #[test]
    fn a_move_under_capture_scrolls_the_list() {
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut p = panel_with_a_bar();
        let inst = &mut p as *mut Appcats as *mut c_void;
        let mut a = untouched();

        (API.drag)(inst, DRAG_BEGIN, 92.0, 0.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_CAPTURE);
        assert_eq!(p.scroll, 0.0);
        (API.drag)(inst, DRAG_MOVE, 92.0, 40.0, r, 100.0, 100.0, &mut a);
        assert!((p.scroll - 150.0).abs() < 0.5, "{}", p.scroll);
        // Past the bottom of the track the offset stops at the bottom of
        // the list, never beyond it.
        (API.drag)(inst, DRAG_MOVE, 92.0, 999.0, r, 100.0, 100.0, &mut a);
        assert_eq!(p.scroll, 300.0);
        // Let go, and a later motion is nobody's.
        (API.drag)(inst, DRAG_END, 92.0, 999.0, r, 100.0, 100.0, &mut a);
        (API.drag)(inst, DRAG_MOVE, 92.0, 0.0, r, 100.0, 100.0, &mut a);
        assert_eq!(p.scroll, 300.0, "a released thumb does not follow the hand");
    }

    /// A press BESIDE the thumb is still the bar's: it pages by one
    /// content box and takes the gesture, so the rows underneath never
    /// see a click the hand did not aim at them.
    #[test]
    fn a_press_beside_the_thumb_pages_and_is_still_ours() {
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut p = panel_with_a_bar();
        let inst = &mut p as *mut Appcats as *mut c_void;
        let mut a = untouched();

        (API.drag)(inst, DRAG_BEGIN, 92.0, 60.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_CAPTURE, "the bar takes the press it did not grab");
        assert_eq!(p.scroll, 100.0);
        (API.drag)(inst, DRAG_END, 92.0, 60.0, r, 100.0, 100.0, &mut a);

        // The next frame draws the thumb further down; a press above it
        // pages back the way it came.
        p.bar = Some(tile::BarGeom {
            thumb: Rect::new(90.0, 40.0, 6.0, 20.0),
            ..p.bar.unwrap()
        });
        let inst = &mut p as *mut Appcats as *mut c_void;
        (API.drag)(inst, DRAG_BEGIN, 92.0, 10.0, r, 100.0, 100.0, &mut a);
        assert_eq!(a.kind, ACTION_CAPTURE);
        assert_eq!(p.scroll, 0.0);
    }

    /// Before the first frame there is no bar, and nothing is taken: the
    /// list must not claim a gesture over geometry it has not drawn.
    #[test]
    fn no_bar_drawn_means_no_press_taken() {
        let mut p = panel(Vec::new());
        assert!(!p.press(92.0, 10.0));
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

#[cfg(test)]
mod token_tests {
    use super::*;

    /// Every token this widget names for itself, spelled as the code
    /// spells it. A name the master does not declare answers u32::MAX
    /// and then zero, and a row of no size looks exactly like a list
    /// nobody wrote — so a typo fails here or nowhere.
    const TOKENS: &[&str] = &[
        "list.row_h",
        "list.gap",
        "list.pad_x",
        "list.glyph",
        "list.glyph_gap",
        "list.status_gap",
        "list.wheel_px",
        "list.corner",
        "list.corner_style",
        "list.label_role",
        "list.status_role",
        "emptystate.y_frac",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
    }

    /// The row's two bindings are followed to families that exist, and
    /// the label's role is NOT the tile grid's caption — which is the
    /// finding: a category row was drawn one grade under what the
    /// master asks a row of prose to be.
    #[test]
    fn a_row_is_set_in_the_roles_the_list_binds_not_the_tiles() {
        nacelle::theme::load();
        let word = |n: &str| {
            nacelle::theme::enum_word_of(nacelle::theme::id(n).expect(n)).expect("no word")
        };
        let label = word("list.label_role");
        let status = word("list.status_role");
        for role in [&label, &status] {
            for suffix in ["size", "min_px", "tracking", "leading", "case", "face"] {
                let name = tile::role_token(role, suffix).expect("a role names its family");
                assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
            }
        }
        let t = nacelle::theme::resolved();
        let size = |role: &str| {
            t.px(nacelle::theme::id(&tile::role_token(role, "size").unwrap()).expect("size"))
        };
        // The three sizes this row used to draw with, and the two it
        // draws with now: the label really moves off the tile caption.
        assert_ne!(size(&label), size(&word("tile.caption_role")));
        assert_ne!(size(&label), size(&status));
    }

    /// The plate behind a row has a radius of its own, so it no longer
    /// borrows the file browser's tile corner — and its cut is a word
    /// the master states, not a shape this file picked.
    #[test]
    fn the_row_plate_carries_the_lists_own_radius_and_cut() {
        nacelle::theme::load();
        let t = nacelle::theme::resolved();
        let radius = t.px(nacelle::theme::id("list.corner").expect("list.corner"));
        assert!(radius > 0.0, "a row plate with no radius is a rectangle");
        let id = nacelle::theme::id("list.corner_style").expect("list.corner_style");
        let word = nacelle::theme::enum_word_of(id).expect("no word");
        assert_ne!(
            tile::corner_style(&word),
            CORNER_SQUARE,
            "list.corner_style resolves to {word}, which this file cannot cut"
        );
    }
}
