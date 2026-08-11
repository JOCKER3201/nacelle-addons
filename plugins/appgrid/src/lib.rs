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
//!
//! Two of this crate's three modules are public on purpose: [`desktop`]
//! is the system's ONE XDG scanner and [`tile`] its ONE tile grid, and
//! the categories widget next door is built out of both rather than out
//! of copies of both.

pub mod desktop;
pub mod tile;

use desktop::AppEntry;
use nacelle::runtime::{
    ActionC, ChromeC, HostApi, PluginApi, RectC, ABI_VERSION, ACTION_NONE,
};
use std::ffi::c_void;
use std::time::Instant;
use tile::{Rect, TileLook, TileTheme};

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
    theme: Option<TileTheme>,
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
    fn look(&mut self, api: &HostApi, ctx: *mut c_void) -> TileLook {
        // ABI 5 is where the token entries live. attach() refuses an
        // older host outright, so this branch is belt and braces for the
        // day the check moves — an old table simply ends before these
        // entries do.
        if api.abi_version < 5 {
            return TileLook::raw();
        }
        let epoch = (api.theme_epoch)(ctx);
        if self.theme.as_ref().map(|t| t.epoch) != Some(epoch) {
            self.theme = Some(TileTheme::resolve(api, epoch));
        }
        match &self.theme {
            Some(t) => TileLook::read(api, ctx, t),
            None => TileLook::raw(),
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        self.follow();
        let look = self.look(api, ctx);
        self.wheel_px = look.wheel_px;

        if self.entries.is_empty() {
            // Nothing found is not an error — a machine can honestly
            // have no menu — so it says so in the caption role rather
            // than in a critical pill.
            let name_px = look.caption_px;
            let name_sp = name_px * look.caption_tracking;
            let text = tile::recase(look.caption_case, "no applications".to_string());
            // `emptystate.y_frac` says where in the box the line sits;
            // the role's own leading is what centres the line box on it
            // rather than hanging it below.
            tile::draw_text(
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
        let grid = tile::layout(&look, area, self.entries.len(), &mut self.scroll);

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
            let Some(trect) = grid.place(area, i) else { continue };
            let rung = if flashing == Some(i) {
                &look.press
            } else if trect.contains(mx, my) {
                &look.hover
            } else {
                &look.idle
            };
            // The container, the stand-in for the icon (the
            // application's initial) and the name under it, all in one
            // call — the tile the categories widget draws as well.
            let mark = tile::initial(&app.name);
            tile::tile_face(api, ctx, &look, trect, rung, &mark, &app.name);
            self.hits.push((trect, i));
        }

        // The scrollbar, drawn at last: a panel that scrolls without
        // saying where it is is a defect.
        tile::scrollbar(api, ctx, &look, area, grid.scroll());
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
