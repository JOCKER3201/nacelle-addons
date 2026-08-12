//! SEARCH panel — a query box over two sources: the installed
//! applications and the files under the home directory.
//!
//! Two sources, and not one more. A search box that lists sources it
//! cannot actually reach is a promise; every row this panel draws comes
//! from something it walked itself, and both walks are bounded.
//!
//! * applications — the XDG scan of `nacelle-launcher-core`, the very
//!   one the APPLICATIONS grid shows, and a chosen application is handed
//!   to init down the same double fork. A second scan or a second launch
//!   path would be a second launcher that drifts from the first.
//! * files — a walk of `$HOME` on a thread of its own, with a hard cap
//!   on depth, on answers and on entries looked at, refusing hidden
//!   directories and symlinks (see [`files`]). A widget that stops the
//!   desktop while it walks a large home is worse than no widget.
//!
//! # What is where
//!
//! * [`rank`] — the relevance model: exact, then prefix, then contains.
//! * [`files`] — the home walk and the thread it runs on.
//! * [`finder`] — the model: the query field, the ranked page, the row
//!   the keyboard is on, and the throttle that turns a burst of typing
//!   into one search. Every key this panel answers is decided there, and
//!   tested there without a window.
//! * [`field`] — the query box's DRAWING, and the one piece of
//!   duplication in this crate: read its header before judging it.
//!
//! # Opening what was found
//!
//! An application is launched here, exactly as the grid launches one:
//! `ActionC` has no code for "run this command", so the widget owns it.
//!
//! A file is opened here TOO, with the freedesktop opener the host would
//! have run — and now that `key` can answer with an `ActionC`, that
//! needs a better argument than "the key path cannot". It is this: the
//! panel activates TWO kinds of thing and only one of them has an
//! action. Taking `ACTION_OPEN_FILE` for the file while the application
//! still goes out of this file would put two owners on one word —
//! activate — and "Enter opens a file differently from how it launches
//! an application" is the bug nobody would think to look for. One act,
//! one path, for both sources; the day `ActionC` can carry a launch,
//! both move to the host together.
//!
//! Every colour, length, duration and word comes from the theme. Nothing
//! here knows what a colour is: a missing token degrades through the raw
//! answers the ABI itself gives, never through a number that used to be
//! the design.

pub mod field;
pub mod files;
pub mod finder;
pub mod rank;

use crate::field::FieldView;
use crate::files::{Limits, Scan};
use crate::finder::{Finder, Outcome};
use crate::rank::Source;
use nacelle::focus::{Key, KeyEv, Mods};
use nacelle::object::text_input::InputMsg;
use nacelle::runtime::{
    keys, ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_CAPTURE,
    ACTION_NONE, DRAG_BEGIN, DRAG_END, DRAG_MOVE,
};
use nacelle::view::list::{self, ListState, ListStyle, ListView};
use nacelle::view::model::{RowBuf, RowModel};
use nacelle::view::paint::{self, RoleLook};
use nacelle::view::scroll::ScrollPhysics;
use nacelle::view::surface::{AbiSurface, Surface};
use nacelle::view::{Hit, Hits};
use nacelle::widget::factory::BuiltinWidget;
use nacelle::Rect;
use nacelle_launcher_core::desktop;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// How often the menu is looked at again — the grid's rule and the
/// grid's reason: the scan only runs when the applications directories'
/// modification times MOVED, so this is the rate of a handful of `stat`
/// calls, not of a walk.
const RESCAN_SECS: u64 = 5;

/// The program that turns a path into whatever opens it. The
/// freedesktop one, which is what the host itself runs for
/// `Action::OpenFile` — see the header.
const OPENER: &str = "xdg-open";

/// What a row IS, in the trailing column, and what sits between a name
/// and where it lies. Words, not look: the theme decides the size, the
/// case and the ink of both.
const KIND_APP: &str = "application";
const KIND_FILE: &str = "file";
const WHERE_SEP: &str = " \u{2014} ";

/// What the panel says when there is nothing to list. Three different
/// nothings, because "type something", "still looking" and "there is no
/// such thing" are three different answers and one line for all three
/// would be a lie in two of them.
const SAY_START: &str = "type to search applications and files";
const SAY_WORKING: &str = "searching\u{2026}";
const SAY_NOTHING: &str = "nothing found";

/// What the empty box invites.
const PLACEHOLDER: &str = "search";

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

/// Opens a path the way the desktop would.
fn open(path: &Path) -> Result<(), String> {
    let prog = desktop::which(OPENER).ok_or_else(|| format!("no {OPENER} on PATH"))?;
    desktop::spawn_detached(&[prog.display().to_string(), path.display().to_string()])
}

/// A row's identity, and the way back from one.
///
/// The key names the SOURCE and not the row's place on the page, so a
/// click that lands after the page was rebuilt — a walk answering
/// between the draw and the press — resolves to the thing that was
/// under the pointer or to nothing at all, never to whatever moved into
/// that position.
fn key_of(s: Source) -> String {
    match s {
        Source::App(i) => format!("a:{i}"),
        Source::File(i) => format!("f:{i}"),
    }
}

fn source_of(key: &str) -> Option<Source> {
    let (tag, n) = key.split_once(':')?;
    let i: usize = n.parse().ok()?;
    match tag {
        "a" => Some(Source::App(i)),
        "f" => Some(Source::File(i)),
        _ => None,
    }
}

/// A path with the home directory written as `~`, which is how a person
/// reads where a file lies. A path outside the home keeps its own head.
fn shorten(p: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| p.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

/// The key event a boundary call means, if any.
///
/// The table of words this used to carry is gone: [`keys::from_name`] is
/// the ABI's OWN spelling of the neutral key set, so the panel now reads
/// the words the host actually sends rather than a second list of them
/// that could drift. It answers `None` for a word from a newer host,
/// which is what a key this build does not know has to be.
///
/// `mods` is the [`nacelle::runtime::MODS_CTRL`] bits. Unknown bits are
/// dropped by [`Mods::from_bits`] rather than kept: a modifier this build
/// does not know being held must not stop a chord it DOES know from
/// matching.
///
/// `text` stays `None`, and that is the entry's own limit rather than
/// this function's: the boundary carries a KEY, so a character is
/// rebuilt from its scalar and a platform that expanded one press into
/// several characters never reaches here at all.
fn key_ev(ch: u32, label: Option<&str>, mods: u32) -> Option<KeyEv> {
    let key = match label {
        Some(l) => keys::from_name(l)?,
        None => {
            let c = char::from_u32(ch).filter(|c| !c.is_control())?;
            Key::Char(c)
        }
    };
    Some(KeyEv {
        key,
        mods: Mods::from_bits((mods & 0xff) as u8),
        repeat: false,
        text: None,
    })
}

/// The label a boundary call passed, as a string — or `None` for a
/// character key, which is what an empty one means on both key entries.
///
/// # Safety
/// `label` must either be null or point at `label_len` readable bytes
/// that stay put for the call. That is what both key entries promise,
/// and it is why this is `unsafe` rather than a tidy helper: a safe
/// function that dereferences a pointer somebody else chose is a safe
/// function that can be made to read anything.
unsafe fn label_of<'a>(label: *const u8, label_len: u32) -> Option<&'a str> {
    if label.is_null() || label_len == 0 {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(label, label_len as usize)).ok()
}

// ----------------------------------------------------------- the widget

/// Everything the panel reads from the theme, once per frame.
struct Look {
    /// `search.gap` — between the query box and the answers.
    gap: f32,
    /// `field.h` — the query box's own height.
    field_h: f32,
    /// `search.debounce_ms`, in seconds, because that is what the host's
    /// clock counts in.
    debounce_s: f32,
    /// `list.row_h + list.gap` — one row's pitch, for keeping the chosen
    /// row on screen when the arrows walk past the edge.
    pitch: f32,
    /// `list.wheel_px` — a notch, cached because a wheel event arrives
    /// with no drawing context to ask the theme through.
    wheel_px: f32,
    /// The empty state's own line.
    empty: RoleLook,
    empty_y: f32,
}

impl Look {
    fn read(sf: &mut impl Surface) -> Look {
        Look {
            gap: sf.px("search.gap").max(0.0),
            field_h: sf.px("field.h").max(0.0),
            debounce_s: sf.px("search.debounce_ms") / 1000.0,
            pitch: (sf.px("list.row_h") + sf.px("list.gap")).max(1.0),
            wheel_px: sf.px("list.wheel_px"),
            empty: paint::bound_role(sf, "emptystate.role", 1.0),
            empty_y: sf.px("emptystate.y_frac"),
        }
    }
}

/// The ranked page, as a row model.
///
/// Pull-based, which is what [`RowModel`] is for: the list materialises
/// the forty rows it can see and never the two hundred it ranked.
struct Page<'a> {
    f: &'a Finder,
    home: Option<&'a Path>,
}

impl RowModel for Page<'_> {
    fn len(&self) -> usize {
        self.f.hits().len()
    }

    fn row(&self, index: usize, out: &mut RowBuf) {
        out.reset();
        let Some(&s) = self.f.hits().get(index) else { return };
        out.key = key_of(s);
        match s {
            Source::App(i) => {
                let Some(a) = self.f.apps().get(i) else { return };
                // What it is called, then where it lies: an application
                // lies in its desktop entry, and that entry's id is the
                // one thing about it that is unique.
                out.label = format!("{}{WHERE_SEP}{}", a.name, a.id);
                out.status = KIND_APP.to_string();
            }
            Source::File(i) => {
                let Some(p) = self.f.files().get(i) else { return };
                let place = p.parent().map(|d| shorten(d, self.home)).unwrap_or_default();
                out.label = format!("{}{WHERE_SEP}{place}", rank::file_name(p));
                out.status = KIND_FILE.to_string();
            }
        }
    }
}

pub struct Search {
    /// The model: query, page, chosen row, throttle.
    finder: Finder,
    /// The query box's between-frame state.
    field: FieldView,
    /// The answer list's between-frame state, and the rectangles the
    /// last draw recorded for the input that arrives without any.
    list: ListState,
    hits: Hits,
    /// The home walk in flight, if one is.
    scan: Option<Scan>,
    /// The home directory, resolved once: the environment does not
    /// change under a running desktop, and a widget that re-read it per
    /// frame would be pretending it might.
    home: Option<PathBuf>,
    /// What the last draw settled on, for the click and the wheel — both
    /// arrive between frames with no geometry and no clock of their own.
    field_r: Rect,
    list_r: Rect,
    pitch: f32,
    physics: ScrollPhysics,
    now: f64,
    /// When the applications directories were last looked at, and what
    /// they said.
    last_look: Instant,
    stamp: u64,
    /// The menu walk in flight, if one is. The applications scan reads
    /// every `.desktop` file on the XDG path, so it belongs on a thread
    /// for the same reason the home walk does: a frame that waits for a
    /// disk is a frame the desktop does not draw. The channel dies with
    /// the thread, and a dropped receiver is the thread's cue to stop
    /// mattering — nothing here needs cancelling.
    menu: Option<std::sync::mpsc::Receiver<Vec<desktop::AppEntry>>>,
    /// The count as last handed to the host's title band, alive until
    /// the next `chrome` call.
    chrome_right: Vec<u8>,
}

impl Search {
    pub fn new() -> Search {
        let apps = desktop::scan();
        eprintln!("search: {} applications found", apps.len());
        Search {
            finder: Finder::new(apps),
            field: FieldView::new(),
            list: ListState::new(),
            hits: Hits::new(),
            scan: None,
            home: files::home(),
            field_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            list_r: Rect::new(0.0, 0.0, 0.0, 0.0),
            pitch: 0.0,
            // Zeroed rather than read: there is no drawing context yet,
            // and a wheel that arrives before the first frame moves
            // nothing, which is honest — there is nothing on screen to
            // move.
            physics: ScrollPhysics {
                wheel_px: 0.0,
                fling_scale: 0.0,
                glide_halflife_ms: 0.0,
                settle_ms: 0.0,
                settle_easing: nacelle::view::scroll::Easing::Linear,
                motion_scale: 0.0,
            },
            now: 0.0,
            last_look: Instant::now(),
            menu: None,
            stamp: desktop::stamp(),
            chrome_right: Vec::new(),
        }
    }

    /// The menu, kept current without a thread and without a watch — the
    /// grid's arrangement, for the grid's reason.
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
        true
    }

    /// The walk's answer, when it has one and it is still the answer to
    /// the question on screen.
    fn collect(&mut self) -> Option<Vec<PathBuf>> {
        let scan = self.scan.as_mut()?;
        let found = scan.take()?;
        // Belt and braces: a scan is dropped the moment the query
        // changes, so its answer cannot outlive its question. An answer
        // checked is one that cannot be the wrong one anyway.
        let mine = scan.query() == self.finder.query();
        self.scan = None;
        mine.then_some(found)
    }

    /// The query has settled: rebuild the page from what is already
    /// known, and send the walk after the rest.
    fn search(&mut self) {
        // Dropping the old scan cancels its walk and throws its answer
        // away, which is exactly what a superseded query wants.
        self.scan = None;
        let q = self.finder.query().to_string();
        if q.is_empty() {
            // An empty box has no answers, so the last walk's files go
            // with the query that asked for them.
            self.finder.set_files(Vec::new());
            return;
        }
        // The files still held are the LAST query's, re-graded against
        // this one — every one of them is a file that exists and whose
        // name matches, so they are shown rather than blanked while the
        // walk catches up. When it answers they are replaced whole.
        if let Some(home) = self.home.clone() {
            self.scan = Some(Scan::start(home, q, Limits::default()));
        }
    }

    /// Keeps the chosen row on screen after the arrows moved it. Uses
    /// the pitch and the viewport of the LAST draw, which is the only
    /// geometry a key event has.
    fn reveal(&mut self) {
        let Some(i) = self.finder.cursor() else { return };
        if self.pitch <= 0.0 || self.list_r.h <= 0.0 {
            return;
        }
        let top = i as f32 * self.pitch;
        let offset = self.list.scroll.offset();
        if top < offset {
            self.list.scroll.set_offset(top);
        } else if top + self.pitch > offset + self.list_r.h {
            self.list.scroll.set_offset(top + self.pitch - self.list_r.h);
        }
    }

    /// Runs what a row points at. The two sources are opened two ways
    /// because they are two different things, not because they arrived
    /// by two different paths — see the module header.
    fn activate(&self, s: Source) {
        match s {
            Source::App(i) => {
                let Some(a) = self.finder.apps().get(i) else { return };
                if let Err(e) = desktop::launch(a) {
                    eprintln!("search: {} \u{2014} {e}", a.name);
                }
            }
            Source::File(i) => {
                let Some(p) = self.finder.files().get(i) else { return };
                match open(p) {
                    Ok(()) => eprintln!("search: opened {}", p.display()),
                    Err(e) => eprintln!("search: {} \u{2014} {e}", p.display()),
                }
            }
        }
    }

    /// A key delivered to this panel. Answers whether it was CONSUMED —
    /// what [`nacelle::runtime::PluginApi::key`] returns, and what stops
    /// the host from also spending it on focus navigation or a shortcut.
    ///
    /// [`Outcome::Ignored`] is the only "no": it is the model's word for
    /// "nothing here answers this key", which is why an arrow at the end
    /// of the list — a request to leave it — reaches the host rather
    /// than being eaten by a panel that did nothing with it.
    pub fn key(&mut self, ev: &KeyEv) -> bool {
        match self.finder.key(ev, self.now) {
            Outcome::Activate(s) => self.activate(s),
            Outcome::Moved => self.reveal(),
            // The page was rebuilt inside the model, and a new page
            // starts at its top. What waits for the throttle is the walk
            // of the home directory, which `draw` starts when it is due.
            Outcome::Edited => self.list.scroll.reset(),
            Outcome::Ignored => return false,
        }
        true
    }

    /// A press. Answers what it activated, if anything.
    pub fn click(&mut self, x: f32, y: f32) -> Option<Source> {
        if self.field_r.contains(x, y) {
            // The caret goes where the pointer is. The offset comes from
            // the positions the last draw recorded, because a click
            // arrives with no way to measure text.
            let at = self.field.hit(x);
            self.finder.apply(InputMsg::Point { at, extend: false }, self.now);
            return None;
        }
        // Copied out before anything is touched: the hit list and the
        // state a hit changes are both this widget's.
        let hit = self.hits.at(x, y).cloned();
        match hit {
            Some(Hit::Row { key, .. }) => {
                let s = source_of(&key)?;
                self.finder.choose(s);
                self.activate(s);
                Some(s)
            }
            Some(Hit::Track { toward_end, .. }) => {
                let viewport = self.list.extent.viewport;
                self.list.scroll.page(toward_end, viewport, self.now);
                None
            }
            _ => None,
        }
    }

    pub fn wheel(&mut self, notches: f32) {
        // Positive `dy` from the host scrolls toward the START of the
        // content; `ScrollView` counts the other way.
        self.list.scroll.wheel(-notches, &self.physics, self.now);
    }

    /// A press that takes hold of the answer list's scroll thumb.
    /// Anything else is not a gesture this panel wants: it declines, and
    /// the press falls back to the ordinary click delivery.
    pub fn grab(&mut self, x: f32, y: f32) -> bool {
        let Some((_, thumb)) = self.list.extent.bar else { return false };
        thumb.contains(x, y) && self.list.scroll.press_thumb(y, thumb)
    }

    /// The pointer moved while holding it.
    pub fn drag_to(&mut self, y: f32) {
        let Some((track, _)) = self.list.extent.bar else { return };
        let (viewport, content) = (self.list.extent.viewport, self.list.extent.content);
        self.list.scroll.drag(y, viewport, content, track);
    }

    /// And let it go.
    pub fn release(&mut self) {
        self.list.scroll.release();
    }

    /// The line a panel with nothing to list draws instead of a hole.
    fn nothing(&self, sf: &mut impl Surface, r: Rect, look: &Look) {
        let say = if self.finder.query().is_empty() {
            SAY_START
        } else if self.finder.armed() || self.scan.is_some() {
            SAY_WORKING
        } else {
            SAY_NOTHING
        };
        // `emptystate.y_frac` says where in the box the line sits; the
        // role's own leading is what centres the line box on it rather
        // than hanging it below.
        let y = r.y + r.h * look.empty_y - look.empty.px * look.empty.leading / 2.0;
        sf.text(
            look.empty.face,
            look.empty.px,
            r.cx(),
            y,
            say,
            look.empty.color,
            look.empty.track,
            nacelle::ui::Align::Center,
        );
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        let mut sf = AbiSurface::new(api, ctx);
        self.hits.clear();
        self.now = sf.now();
        let look = Look::read(&mut sf);
        self.pitch = look.pitch;
        self.physics = ScrollPhysics::read(&mut sf);
        // A row list names its own notch; the generic `scroll.wheel_px`
        // is a different distance for a different thing. A theme that
        // names none leaves the notch at one row, which is the smallest
        // move that still means something.
        self.physics.wheel_px = look.wheel_px.max(look.pitch);

        if self.follow() && self.menu.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(desktop::scan());
            });
            self.menu = Some(rx);
        }
        if let Some(rx) = &self.menu {
            match rx.try_recv() {
                Ok(apps) => {
                    eprintln!("search: menu changed \u{2014} {} applications", apps.len());
                    self.finder.set_apps(apps);
                    self.menu = None;
                }
                // The walk is still out. Disconnected means the thread
                // died without answering; either way this frame draws
                // what it already has rather than waiting for a disk.
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.menu = None,
            }
        }
        if let Some(found) = self.collect() {
            self.finder.set_files(found);
        }
        if self.finder.due(self.now, look.debounce_s) {
            self.search();
        }

        // The box on top, its answers under it.
        let field_r = Rect::new(r.x, r.y, r.w, look.field_h.min(r.h));
        let top = field_r.bottom() + look.gap;
        let list_r = Rect::new(r.x, top, r.w, (r.bottom() - top).max(0.0));
        self.field_r = field_r;
        self.list_r = list_r;

        // There is no focus chain across this boundary, so the panel
        // answers the question the chain would: a search box owns the
        // keyboard for as long as it is on screen. That is also what it
        // means for the caret to be drawn at all.
        field::draw(&mut sf, field_r, &self.finder.input, &mut self.field, PLACEHOLDER, true);

        if self.finder.hits().is_empty() {
            self.nothing(&mut sf, list_r, &look);
            return;
        }
        // The list's selection is the model's, restated every frame: the
        // page is rebuilt whenever either source moves, and a key kept
        // from before that would name a row nobody chose.
        self.list.selected = self.finder.chosen().map(key_of);
        let page = Page { f: &self.finder, home: self.home.as_deref() };
        list::list(
            &mut sf,
            list_r,
            &page,
            &ListStyle::default(),
            Some(ListView {
                state: &mut self.list,
                hits: &mut self.hits,
                id: 0,
                select: true,
                scroll: true,
                tree: false,
                // A row name the ellipsis cut short finishes itself
                // under the pointer. This panel is where that matters
                // most: the answers are whatever the query found, so a
                // long application name and a deep path are the ordinary
                // case rather than the awkward one, and a row that ends
                // in an ellipsis is an answer the reader cannot check.
                // The box is the HOST's — filed over
                // `HostApi::tooltip`, drawn last of everything, because
                // a plugin paints in the middle of the frame and
                // anything it drew outside its own rectangle would go
                // under the panels drawn after it.
                tooltip: true,
            }),
        );
    }
}

impl Default for Search {
    fn default() -> Self {
        Search::new()
    }
}

// ----------------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Search::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        // The walk in flight goes with it: `Scan`'s own drop sets the
        // cancel flag, and its thread ends at the next batch.
        drop(unsafe { Box::from_raw(instance as *mut Search) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Search> {
    unsafe { (instance as *mut Search).as_mut() }
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
        let _ = this.click(x, y);
    }
    if let Some(out) = unsafe { out.as_mut() } {
        // Whatever was chosen is already on its way; the host has
        // nothing to do about it.
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
        this.wheel(dy);
    }
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = ACTION_NONE;
    }
}

extern "C" fn grid_c(_: *mut c_void, _: *mut u32, _: *mut u32) {}

/// Whether this host routes keys to the widget that owns the keyboard.
///
/// Nothing answers that directly: [`PluginApi::key`] is the PLUGIN's
/// table, and a plugin cannot see how the host reads its own table. So
/// the question is put to the HOST's, where `channel_read` was appended
/// by the same change in the same version — a host whose `api_size`
/// reaches one reaches both.
///
/// It has to be asked at all because `key_feedback` is a BROADCAST that
/// arrives whatever else the host does. On a host that routes, acting on
/// both would type every character twice.
fn routes_keys(api: &HostApi) -> bool {
    api.has_channel()
}

/// The BROADCAST, which this panel now listens to only where nothing
/// else is delivered.
///
/// The entry is what it always was — every widget hears every key, so an
/// on-screen keyboard can light up what somebody else is typing — and it
/// is the wrong channel for a query box in three ways: it reaches every
/// instance, so two search panels on one board would eat the same
/// character; it carries no modifiers, so the field's own chords are
/// unreachable; and it cannot answer, so a key it used is spent again by
/// the host. [`key_c`] is the entry with none of those, and where the
/// host offers it this one steps aside rather than doubling it.
///
/// What is left here is the older host's path, kept because it works:
/// the panel typed through the broadcast before `key` existed, and a
/// host that never calls `key` would otherwise have a query box with no
/// keyboard at all.
extern "C" fn key_feedback_c(
    instance: *mut c_void,
    ch: u32,
    label: *const u8,
    label_len: u32,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    if routes_keys(api) {
        return;
    }
    // No modifier state crosses this entry, so the chords stay out of
    // reach here — guessing which one a bare character came from would
    // be inventing input the user did not give.
    // Safety: the entry's contract — a null pointer or `label_len`
    // readable bytes that outlive the call.
    let label = unsafe { label_of(label, label_len) };
    if let Some(ev) = key_ev(ch, label, 0) {
        this.key(&ev);
    }
}

/// The key delivered to the widget that OWNS the keyboard, modifiers and
/// all — the entry this panel was written against before it existed.
///
/// What a key MEANS is still decided in one tested place
/// ([`finder::Finder::key`]) rather than at the boundary; what this
/// function adds is the answer. Nonzero says the panel used the key, so
/// the host must not also spend it on focus navigation, a shortcut or
/// the shell's bytes: a character typed into the query box is not also a
/// jump to the next panel.
///
/// `out` is left at [`ACTION_NONE`] on every path. Enter runs what was
/// found, and both kinds of thing this panel finds are run from here —
/// see the module header for why that is one path rather than two.
extern "C" fn key_c(
    instance: *mut c_void,
    ch: u32,
    label: *const u8,
    label_len: u32,
    mods: u32,
    out: *mut ActionC,
) -> u32 {
    if let Some(out) = unsafe { out.as_mut() } {
        out.kind = ACTION_NONE;
    }
    let Some(this) = state(instance) else { return 0 };
    // Safety: the entry's contract — a null pointer or `label_len`
    // readable bytes that outlive the call.
    let label = unsafe { label_of(label, label_len) };
    let Some(ev) = key_ev(ch, label, mods) else {
        // A key this build cannot name and cannot spell is not one the
        // panel used; leaving it to the host is the honest answer.
        return 0;
    };
    this.key(&ev) as u32
}

/// Grows downwards: a taller panel is more answers, not bigger ones.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_ROWS
}

/// The header: the panel's name on the left, how many answers on the
/// right — the same two strings the title band would have had to be told
/// anyway.
extern "C" fn chrome_c(
    instance: *mut c_void,
    _ctx: *mut c_void,
    _host_data: *const c_void,
    out: *mut ChromeC,
    out_size: u32,
) -> u32 {
    static TITLE: &[u8] = b"SEARCH";
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return 0;
    };
    this.chrome_right = this.finder.hits().len().to_string().into_bytes();
    out.title = TITLE.as_ptr();
    out.title_len = TITLE.len() as u32;
    out.right = this.chrome_right.as_ptr();
    out.right_len = this.chrome_right.len() as u32;
    (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
}

/// The one gesture this panel takes: the answer list's scroll thumb.
/// Every other press declines the capture and falls back to the click
/// path, which is what puts the caret and what runs a row.
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
                kind = if this.grab(x, y) { ACTION_CAPTURE } else { ACTION_NONE };
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

/// Filled, and does nothing on purpose. The one gesture this panel takes
/// is the thumb, and the thumb is `drag`'s — the single capture path, of
/// which this entry is deliberately not a second. What is left for it is
/// the press RUNG, and no control here draws one: the rows are a list,
/// and the thumb's rungs are idle, hover and dragging, which the last
/// frame's own state already says.
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
/// `search.so` from the addons directory. The name and the metadata are
/// the addon's own — the same string the file would be called and the
/// very bytes of `search.meta` beside it — so a host never describes a
/// widget it merely links.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "search",
    meta: include_str!("../search.meta"),
    attach: builtin_attach,
};

/// In-process attach for a host that links this crate statically. The
/// dlopen attach below goes through `runtime::attach`, which flips the
/// toolkit into forwarding mode — correct for a plugin carrying its own
/// copy of the toolkit, and exactly wrong when this copy IS the host's.
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

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nacelle_launcher_core::desktop::AppEntry;

    /// Every token this crate names, spelled exactly as the code spells
    /// it — the whole of `field.rs`, `Look::read` and the row model.
    ///
    /// This is the test that makes "no hardcoded values" a FACT rather
    /// than a promise. A widget that names a token the master does not
    /// declare gets `u32::MAX` back, `theme_px` answers zero, and the
    /// thing degrades silently: a caret of no width and a gap of no
    /// height look exactly like a search box nobody finished. A typo
    /// would therefore never fail loudly anywhere else — so it fails
    /// here.
    ///
    /// The type roles themselves are NOT listed: they are reached
    /// through `view::paint`, which the toolkit's own tests cover, and
    /// the binding tokens that name them (`field.role`, `emptystate.role`
    /// and the two `list.*_role`s) are.
    const TOKENS: &[&str] = &[
        // the panel's own two
        "search.gap",
        "search.debounce_ms",
        // the query box — [field] and its component colours
        "field.h",
        "field.corner",
        "field.pad_x",
        "field.border",
        "field.border_focused",
        "field.role",
        "field.scroll_margin",
        "field.caret_w",
        "field.caret_h",
        "field.caret_style",
        "field.preedit_underline",
        "component.field.fill",
        "component.field.border",
        "component.field.text",
        "component.field.placeholder",
        "component.field.caret",
        "component.field.selection",
        "component.field.selection_text",
        "component.field.preedit",
        // the caret's blink
        "motion.scale",
        "motion.caret_blink.enabled",
        "motion.caret_blink.period_ms",
        "motion.caret_blink.duty",
        // the answers
        "list.row_h",
        "list.gap",
        "list.wheel_px",
        "list.label_role",
        "list.status_role",
        // and the panel with nothing to answer
        "emptystate.role",
        "emptystate.y_frac",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
        // The classes the two halves rest on. A class the matrix does not
        // know answers the raw rung, and the panel would look undesigned
        // for a reason no reader could see.
        for class in ["field", "list.item", "scrollbar.thumb"] {
            assert!(nacelle::theme::class_id(class).is_some(), "no class.{class}");
        }
    }

    #[test]
    fn a_row_says_what_it_is_and_where_it_lies() {
        let home = PathBuf::from("/home/u");
        let mut f = Finder::new(vec![AppEntry {
            id: "firefox.desktop".into(),
            name: "Firefox".into(),
            exec: "/bin/true".into(),
            terminal: false,
            icon: String::new(),
            categories: Vec::new(),
        }]);
        f.set_files(vec![PathBuf::from("/home/u/Documents/report.pdf")]);
        f.input.set_value("f");
        f.rerank();
        let page = Page { f: &f, home: Some(home.as_path()) };
        let mut row = RowBuf::new();
        assert_eq!(page.len(), 2);
        page.row(0, &mut row);
        assert_eq!(row.label, "Firefox \u{2014} firefox.desktop");
        assert_eq!(row.status, KIND_APP);
        assert_eq!(source_of(&row.key), Some(Source::App(0)));
        page.row(1, &mut row);
        // The home directory reads as `~`, which is where a person
        // thinks their files are.
        assert_eq!(row.label, "report.pdf \u{2014} ~/Documents");
        assert_eq!(row.status, KIND_FILE);
        assert_eq!(source_of(&row.key), Some(Source::File(0)));
        // A row past the end is an empty buffer, never a panic.
        page.row(9, &mut row);
        assert!(row.key.is_empty() && row.label.is_empty());
    }

    #[test]
    fn a_row_key_names_the_source_and_survives_the_round_trip() {
        for s in [Source::App(0), Source::App(17), Source::File(3)] {
            assert_eq!(source_of(&key_of(s)), Some(s));
        }
        // Nothing else is a key: a hit recorded by a view this widget
        // does not own must resolve to nothing rather than to row zero.
        assert_eq!(source_of(""), None);
        assert_eq!(source_of("a"), None);
        assert_eq!(source_of("x:1"), None);
        assert_eq!(source_of("a:-1"), None);
        assert_eq!(source_of("a:"), None);
    }

    #[test]
    fn a_path_reads_from_the_home_it_is_under() {
        let home = Path::new("/home/u");
        assert_eq!(shorten(Path::new("/home/u/a/b"), Some(home)), "~/a/b");
        assert_eq!(shorten(Path::new("/home/u"), Some(home)), "~");
        // Outside the home, and with no home at all, a path is itself.
        assert_eq!(shorten(Path::new("/etc/hosts"), Some(home)), "/etc/hosts");
        assert_eq!(shorten(Path::new("/home/u/a"), None), "/home/u/a");
    }

    #[test]
    fn the_key_channel_translates_what_it_can_and_refuses_the_rest() {
        let ev = |ch, label| key_ev(ch, label, 0);
        assert_eq!(ev('a' as u32, None).map(|e| e.key), Some(Key::Char('a')));
        assert_eq!(ev(0, Some("ENTER")).map(|e| e.key), Some(Key::Enter));
        assert_eq!(ev(0, Some("esc")).map(|e| e.key), Some(Key::Escape));
        // The words the host could never send before this entry, every
        // one of which the panel has something to do with.
        assert_eq!(ev(0, Some("DOWN")).map(|e| e.key), Some(Key::Down));
        assert_eq!(ev(0, Some("UP")).map(|e| e.key), Some(Key::Up));
        assert_eq!(ev(0, Some("LEFT")).map(|e| e.key), Some(Key::Left));
        assert_eq!(ev(0, Some("RIGHT")).map(|e| e.key), Some(Key::Right));
        assert_eq!(ev(0, Some("HOME")).map(|e| e.key), Some(Key::Home));
        assert_eq!(ev(0, Some("END")).map(|e| e.key), Some(Key::End));
        assert_eq!(ev(0, Some("DELETE")).map(|e| e.key), Some(Key::Delete));
        // A name this build does not know is not a key, and neither is a
        // control character: guessing at either would type something the
        // user did not press.
        assert!(ev(0, Some("F13")).is_none());
        assert!(ev(0, Some("")).is_none());
        assert!(ev(0x1b, None).is_none());
        assert!(ev(0, None).is_none());

        // The modifiers, which are the whole reason for the second key
        // entry: the bits cross as a number and rebuild as the set.
        use nacelle::runtime::{MODS_ALT, MODS_CTRL, MODS_SHIFT};
        let mods = |m| key_ev('a' as u32, None, m).map(|e| e.mods);
        assert_eq!(mods(0), Some(Mods::NONE));
        assert_eq!(mods(MODS_CTRL), Some(Mods::CTRL));
        assert_eq!(mods(MODS_CTRL | MODS_SHIFT), Some(Mods::CTRL | Mods::SHIFT));
        assert_eq!(mods(MODS_ALT), Some(Mods::ALT));
        // A bit from a newer host is DROPPED, not kept: an unknown
        // modifier held down must not stop a chord this build does know
        // from matching.
        assert_eq!(mods(MODS_CTRL | 1 << 20), Some(Mods::CTRL));

        // A label pointer, as the boundary really passes one: empty and
        // null are both "this is a character key", and bytes that are
        // not UTF-8 are no key at all.
        unsafe {
            assert_eq!(label_of(b"UP".as_ptr(), 2), Some("UP"));
            assert_eq!(label_of(b"UP".as_ptr(), 0), None);
            assert_eq!(label_of(std::ptr::null(), 4), None);
            assert_eq!(label_of([0xffu8, 0xfe].as_ptr(), 2), None);
        }
    }

    /// A panel, without a window, a theme or a font: everything below is
    /// decided in the model, which is what makes it testable at all.
    fn panel(apps: Vec<AppEntry>, files: Vec<PathBuf>) -> Search {
        let mut s = Search::new();
        s.finder = Finder::new(apps);
        s.finder.set_files(files);
        s
    }

    fn app(name: &str) -> AppEntry {
        AppEntry {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.to_string(),
            exec: "/bin/true".into(),
            terminal: false,
            icon: String::new(),
            categories: Vec::new(),
        }
    }

    /// One key, as the host's table delivers it: a character or a word,
    /// plus the modifier bits.
    fn press(s: &mut Search, ch: char, label: Option<&str>, mods: u32) -> bool {
        let ev = key_ev(ch as u32, label, mods).expect("a key this build knows");
        s.key(&ev)
    }

    #[test]
    fn the_chords_the_broadcast_could_never_carry_reach_the_query_box() {
        let mut s = panel(vec![app("Firefox"), app("Files")], Vec::new());
        use nacelle::runtime::MODS_CTRL;

        // Typing, which worked before and still does.
        for c in "fire".chars() {
            assert!(press(&mut s, c, None, 0), "a character is the panel's");
        }
        assert_eq!(s.finder.query(), "fire");

        // Ctrl+A then a character replaces the whole query — the select
        // all that was unreachable while no entry carried a modifier.
        assert!(press(&mut s, 'a', None, MODS_CTRL));
        assert!(press(&mut s, 'x', None, 0));
        assert_eq!(s.finder.query(), "x");

        // Undo, and redo, which are the same story.
        assert!(press(&mut s, 'z', None, MODS_CTRL));
        assert_eq!(s.finder.query(), "fire", "undo puts back what was replaced");
        assert!(press(&mut s, 'y', None, MODS_CTRL));
        assert_eq!(s.finder.query(), "x");

        // A chord the field has no meaning for is NOT consumed: it
        // belongs to the host's shortcut registry, and a panel that ate
        // it would make a desktop-wide binding stop working wherever a
        // search box happened to have the keyboard.
        assert!(!press(&mut s, 'q', None, MODS_CTRL));
        assert_eq!(s.finder.query(), "x", "and types nothing either");

        // The clipboard chords reach the field and the field asks for
        // the clipboard — which no ABI entry hands a plugin. The model
        // answers the intent with a shrug rather than pretending, so the
        // key is not consumed and the host's own copy and paste keep
        // working over this panel. That is a LIMIT of this version, not
        // of the entry: `mods` arrives, the chord is recognised, and
        // there is simply nothing on the other end of it yet.
        assert!(!press(&mut s, 'c', None, MODS_CTRL));
        assert!(!press(&mut s, 'v', None, MODS_CTRL));
        assert_eq!(s.finder.query(), "x");
    }

    #[test]
    fn the_navigation_keys_walk_the_answers_and_the_caret() {
        let mut s = panel(
            vec![app("Firefox"), app("Firewall"), app("Firmware")],
            Vec::new(),
        );
        for c in "fir".chars() {
            press(&mut s, c, None, 0);
        }
        assert_eq!(s.finder.hits().len(), 3);
        assert_eq!(s.finder.cursor(), Some(0));

        // The arrows walk the answer list — the keys the broadcast never
        // sent at all, so this is the first time the panel can be driven
        // without the pointer.
        assert!(press(&mut s, '\0', Some("DOWN"), 0));
        assert_eq!(s.finder.cursor(), Some(1));
        assert!(press(&mut s, '\0', Some("DOWN"), 0));
        assert_eq!(s.finder.cursor(), Some(2));
        // Clamped at the end, and NOT consumed there: an arrow that
        // changed nothing is the request to leave the list that it is,
        // and the host's focus chain is what answers it.
        assert!(!press(&mut s, '\0', Some("DOWN"), 0));
        assert_eq!(s.finder.cursor(), Some(2));
        assert!(press(&mut s, '\0', Some("UP"), 0));
        assert_eq!(s.finder.cursor(), Some(1));

        // HOME and END are the FIELD's, because a one-line field has
        // nowhere vertical to go and the list is walked with the arrows:
        // they move the caret, and the chosen row stays where it was.
        assert!(press(&mut s, '\0', Some("HOME"), 0));
        assert_eq!(s.finder.input.cursor(), 0);
        assert_eq!(s.finder.cursor(), Some(1), "the answer list did not move");
        assert!(press(&mut s, '\0', Some("END"), 0));
        assert_eq!(s.finder.input.cursor(), "fir".len());

        // DELETE forward from HOME, which is the other key that used to
        // arrive as nothing.
        press(&mut s, '\0', Some("HOME"), 0);
        assert!(press(&mut s, '\0', Some("DELETE"), 0));
        assert_eq!(s.finder.query(), "ir");

        // Escape empties the box; a second Escape is not the panel's,
        // because it belongs to whatever put the panel on screen.
        assert!(press(&mut s, '\0', Some("ESC"), 0));
        assert_eq!(s.finder.query(), "");
        assert!(!press(&mut s, '\0', Some("ESC"), 0));

        // And Enter over nothing is nobody's key either.
        assert!(!press(&mut s, '\0', Some("ENTER"), 0));
    }

    /// The board this panel may be placed on, read the way the host
    /// reads it: from the addon's own `.meta`, through the registry's
    /// parser, not from anything this file asserts about itself.
    ///
    /// An unknown or absent category word degrades to BOARD, so a typo
    /// here would not fail — it would quietly offer the search panel on
    /// HOME, where it does not belong. That silence is what this test
    /// exists to break.
    #[test]
    fn the_widget_registers_on_the_search_and_ai_board() {
        use nacelle::base::WidgetCategory;
        use nacelle::widget::registry;
        let def = registry::def_from_meta(WIDGET.name.to_string(), WIDGET.meta);
        assert_eq!(def.name, "search");
        assert_eq!(def.label, "SEARCH");
        assert_eq!(def.category, WidgetCategory::SearchAi);
        assert_ne!(WidgetCategory::default(), WidgetCategory::SearchAi);
        assert!(def.ref_h_vh > 0.0 && def.min_h_vh > 0.0);
        assert!(def.min_h_vh <= def.ref_h_vh);
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

    /// The entries appended in this version are filled AND declared, and
    /// this is the widget where one of them does something.
    ///
    /// Filled and declared are two different things, and the host checks
    /// the second: it reads `api_size` before it calls either, so a table
    /// that carried the pointers without reaching them would be a widget
    /// the host never asks.
    ///
    /// Driven through the TABLE rather than through `Search::key`, so
    /// what is proved is the boundary itself — the pointers, the labels
    /// as bytes with a length, the modifier bits as a number, and the
    /// answer the host reads.
    #[test]
    fn the_appended_entries_are_declared_and_the_key_one_answers() {
        assert_eq!(API.api_size as usize, std::mem::size_of::<PluginApi>());
        assert!(API.api_size as usize >= PLUGIN_API_HAS_BUTTON);

        let inst = (API.create)();
        assert!(!inst.is_null());
        let mut a = untouched();

        // A character is the panel's, and the answer says so — which is
        // what stops the host spending it a second time on itself.
        assert_eq!((API.key)(inst, 'f' as u32, std::ptr::null(), 0, 0, &mut a), 1);
        assert_eq!(a.kind, ACTION_NONE, "consumed, and nothing asked of the application");
        // A chord the field understands, over the boundary as bits.
        a = untouched();
        assert_eq!((API.key)(inst, 'a' as u32, std::ptr::null(), 0, MODS_CTRL, &mut a), 1);
        // TAB is NOT the panel's: the host must still be able to move the
        // focus off a search box, and a panel that ate it would trap the
        // keyboard where the user put it once.
        let tab = keys::TAB;
        assert_eq!((API.key)(inst, 0, tab.as_ptr(), tab.len() as u32, 0, &mut a), 0);
        // Nor is a word this build cannot name.
        let unknown = "F13";
        assert_eq!(
            (API.key)(inst, 0, unknown.as_ptr(), unknown.len() as u32, 0, &mut a),
            0
        );

        // The press entry is filled and deliberately empty — see
        // `button_c` for why the gesture is `drag`'s alone.
        let r = RectC { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut b = untouched();
        for phase in [BUTTON_PRESS, BUTTON_RELEASE] {
            (API.button)(inst, phase, 1.0, 1.0, r, 100.0, 100.0, &mut b);
        }
        assert_eq!(b.kind, u32::MAX, "an entry that does nothing writes nothing");

        (API.destroy)(inst);
    }
}
