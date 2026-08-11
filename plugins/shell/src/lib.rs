//! The terminal view, as a compiled widget.
//!
//! The last widget that could not be a script, and not for the reason
//! people assume. Speed is part of it — this draws a rectangle and a
//! glyph per character cell, thousands of them a frame — but the real
//! obstacle is that the terminal's state is Rust types that cannot cross
//! a library boundary at all.
//!
//! So the host resolves the visible grid and hands it over in one call:
//! `term_view` fills a buffer of cells whose colours are already final.
//! Nothing here reimplements what bold does to a colour index or what an
//! unset background means — those are the emulator's rules, and a second
//! copy of them would be a shade that is quietly wrong.
//!
//! What is still this widget's business is the drawing itself — the
//! slanted tabs, the order the primitives go down in, where the cursor
//! block sits and how it blinks, the SCROLL pill. The frame is not: the
//! container is the HOST's (u2 §4), and `r` arrives as the content box
//! it left. Since ABI 5 the LOOK of all of it is read, not known: every
//! colour, length and state rung comes from the host's theme by token
//! name, and the only appearance this file owns is the raw grey a
//! missing theme answers.

use nacelle::runtime::{
    ActionC, CellC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, TermReqC,
    TermSelectC, TermViewC, ABI_VERSION, ACTION_NONE, ACTION_SCROLL_TERMINAL,
    ACTION_SELECT_TAB, ACTION_TERM_SELECT, CELL_ABSENT, CELL_HAS_BG, CELL_SELECTED,
    CORNER_CHAMFER, CORNER_ROUND, CORNER_SQUARE,
    CELL_UNDERLINE, DRAG_BEGIN, DRAG_END, SELECT_KIND_CELLS, SELECT_OP_BEGIN, SELECT_OP_END,
    SELECT_OP_EXTEND, VIEW_CURSOR, VIEW_LIVE, VIEW_TRUNCATED,
};
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};

const FONT_UI: u32 = 0;

/// The interface level the theme entries appeared at. Checked against
/// the HOST's table, not our own [`ABI_VERSION`]: linked statically the
/// two are the same by construction, but a dlopen host may be older than
/// this crate, and reading an entry it never wrote runs off the end of
/// its table.
const THEME_ABI: u32 = 5;

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
}

// ------------------------------------------------------------------ theme
//
// The theme crosses the boundary as tokens: a dotted NAME resolves to an
// id once, and the id is read every frame. Names are the stable
// contract; ids are per-master-load, so the whole table below is
// re-resolved whenever `theme_epoch` moves. A token the master does not
// declare resolves to u32::MAX, and every accessor then answers the
// engine's raw default for its KIND — mid grey ink, 0.0 for a length —
// which is the unstyled look, never a value that used to be the design.

/// The engine's raw ink: what a missing colour answers, and what every
/// colour becomes on a host too old to carry the theme entries.
const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };

/// The engine's raw rung — `StateStyle::RAW`, mirrored rather than
/// imported because the host's copy of the engine owns the real one.
/// No fill, grey ink, one hairline: kind defaults, not a design.
const RAW_STATE: StateStyleC = StateStyleC {
    fill: ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
    edge: RAW_INK,
    text: RAW_INK,
    glyph: RAW_INK,
    edge_width: 1.0,
    glow_radius: 0.0,
    glow_alpha: 0.0,
    elevation: 0.0,
};

/// Rows of the class x state matrix, in the ladder's declaration order.
/// The two this widget never enters (press, dragging) have no index here
/// on purpose: a tab is selected, hovered, resting — or EMPTY, which is
/// a disabled control, not a dimmer kind of occupied.
const ST_IDLE: u32 = 0;
const ST_HOVER: u32 = 1;
const ST_SELECTED: u32 = 3;
const ST_SELECTED_HOVER: u32 = 4;
const ST_DISABLED: u32 = 6;

/// Every token this widget reads, resolved once per theme epoch.
#[derive(Clone, Copy, Default)]
struct Tokens {
    epoch: u32,
    // No frame tokens: the container — material, ring, chamfer — is the
    // HOST's since u2 §4, drawn before this widget is called, and `r` is
    // the content box it left. The chamfer this file used to draw was
    // the second copy of the retired hardcoded frame (u2 §2.9 item 1).
    // the session tab strip
    tab_class: u32, // the "tab" row of the state matrix
    tab_pad: u32,   // tab.pad
    tab_h: u32,     // tab.h
    tab_gap: u32,   // tab.gap
    tab_skew: u32,  // tab.skew — 0 in the master: a tab wears the frames' corners
    tab_corner: u32,
    tab_corner_style: u32,
    tab_count: u32, // tab.count — the strip before the host has said
    rule_c: u32,    // tab.rule_color — the line under the whole strip
    rule_w: u32,    // tab.rule
    rule_gap: u32,  // tab.rule_gap
    label_size: u32,  // type.button.size — tab.role is button
    label_min: u32,   // type.button.min_px
    label_track: u32, // type.button.tracking, in em
    label_lead: u32,  // type.button.leading
    label_case: u32,  // type.button.case
    center_mode: u32, // rhythm.center_mode
    center_bias: u32, // rhythm.cap_center_bias, a fraction of the px
    // the cell grid's chrome (the cells themselves arrive coloured)
    term_pad: u32,    // terminal.pad
    cell_ul_h: u32,   // terminal.cell_underline_h — SGR 4, not the cursor
    cell_ul_gap: u32, // terminal.cell_underline_gap
    wheel_lines: u32, // terminal.wheel_lines
    // the cursor's blink (its colours are the host's, via TermViewC)
    blink_on: u32,     // motion.term_cursor_blink.enabled
    blink_period: u32, // motion.term_cursor_blink.period_ms
    blink_duty: u32,   // motion.term_cursor_blink.duty
    // the SCROLL +n readout — a status pill in a corner, which is
    // precisely the images' badge (u2 §2.9): severity.info's colours,
    // the badge role's type, the badge component's box
    ind_inset: u32,    // terminal.indicator.inset
    badge_size: u32,   // type.badge.size — badge.role is badge
    badge_min: u32,    // type.badge.min_px
    badge_track: u32,  // type.badge.tracking, in em
    badge_lead: u32,   // type.badge.leading
    badge_case: u32,   // type.badge.case
    badge_h: u32,      // badge.h
    badge_pad: u32,    // badge.pad_x
    badge_corner: u32, // badge.corner — `pill` bakes negative and squares off
    badge_border: u32, // badge.border
    info_fill: u32,    // severity.info.fill — the hollow pill's bed
    info_edge: u32,    // severity.info.edge
    info_text: u32,    // severity.info.text
    info_on: u32,      // severity.info.on — the solid pill's ink
    // the selection (F1 §2.4): the flag arrives per cell from the host
    // (CELL_SELECTED); the LOOK is entirely these declared tokens
    sel_wash: u32, // term.selection — the wash behind selected cells
    sel_fg: u32,   // term.selection_fg — glyphs inside a tinted selection
    sel_pad: u32,  // terminal.selection_pad — bleed around the wash quad
    term_bg: u32,  // term.bg — the inverted glyph over a bg-less cell
    /// Whether the pill draws solid — `severity.info.badge_style`'s WORD
    /// (ABI 6), no longer this arrangement's guess. Solid mirrors
    /// `ui::badge`: the severity's text colour becomes the bed and `on`
    /// the ink. `hatched` and `hollow_dashed` degrade to hollow, as the
    /// host degrades them; no word at all (an old host, a missing token)
    /// keeps the pre-word guess: hollow, the master's own info style.
    pill_solid: bool,
    /// Whether the selection TINTS — `term.selection.mode`'s WORD is
    /// `tint` (wash + `selection_fg` glyphs). Anything else — `invert`,
    /// a word this build predates, no word at all — INVERTS, the
    /// master's own default and the reading that never loses a glyph.
    sel_tint: bool,
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

impl Tokens {
    fn resolve(api: &HostApi, ctx: *mut c_void) -> Tokens {
        let t = |n: &str| (api.theme_token)(n.as_ptr(), n.len() as u32);
        let c = |n: &str| (api.theme_class)(n.as_ptr(), n.len() as u32);
        let style = enum_word(api, ctx, t("severity.info.badge_style"));
        let sel_mode = enum_word(api, ctx, t("term.selection.mode"));
        Tokens {
            epoch: (api.theme_epoch)(ctx),
            tab_class: c("tab"),
            tab_pad: t("tab.pad"),
            tab_h: t("tab.h"),
            tab_gap: t("tab.gap"),
            tab_skew: t("tab.skew"),
            tab_corner: t("tab.corner"),
            tab_corner_style: t("tab.corner_style"),
            tab_count: t("tab.count"),
            rule_c: t("tab.rule_color"),
            rule_w: t("tab.rule"),
            rule_gap: t("tab.rule_gap"),
            label_size: t("type.button.size"),
            label_min: t("type.button.min_px"),
            label_track: t("type.button.tracking"),
            label_lead: t("type.button.leading"),
            label_case: t("type.button.case"),
            center_mode: t("rhythm.center_mode"),
            center_bias: t("rhythm.cap_center_bias"),
            term_pad: t("terminal.pad"),
            cell_ul_h: t("terminal.cell_underline_h"),
            cell_ul_gap: t("terminal.cell_underline_gap"),
            wheel_lines: t("terminal.wheel_lines"),
            blink_on: t("motion.term_cursor_blink.enabled"),
            blink_period: t("motion.term_cursor_blink.period_ms"),
            blink_duty: t("motion.term_cursor_blink.duty"),
            ind_inset: t("terminal.indicator.inset"),
            badge_size: t("type.badge.size"),
            badge_min: t("type.badge.min_px"),
            badge_track: t("type.badge.tracking"),
            badge_lead: t("type.badge.leading"),
            badge_case: t("type.badge.case"),
            badge_h: t("badge.h"),
            badge_pad: t("badge.pad_x"),
            badge_corner: t("badge.corner"),
            badge_border: t("badge.border"),
            info_fill: t("severity.info.fill"),
            info_edge: t("severity.info.edge"),
            info_text: t("severity.info.text"),
            info_on: t("severity.info.on"),
            sel_wash: t("term.selection"),
            sel_fg: t("term.selection_fg"),
            sel_pad: t("terminal.selection_pad"),
            term_bg: t("term.bg"),
            pill_solid: style == "solid",
            sel_tint: sel_mode == "tint",
        }
    }
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

// ------------------------------------------------------------------ draw

fn contains(r: &RectC, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    api: &HostApi,
    ctx: *mut c_void,
    font: u32,
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
        font,
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

/// One glyph of the terminal font.
///
/// The character is encoded onto the stack rather than into a `String`:
/// this runs once per non-blank cell, and a full screen would otherwise
/// be twelve thousand heap allocations a frame.
fn glyph(
    api: &HostApi,
    ctx: *mut c_void,
    ch: u32,
    font: u8,
    x: f32,
    y: f32,
    px: f32,
    c: ColorC,
) {
    // Never a transmute: an invalid scalar value is undefined behaviour,
    // and this number crossed a library boundary.
    let Some(ch) = char::from_u32(ch) else { return };
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    draw_text(api, ctx, font as u32, px, x, y, s, c, 0.0, 0);
}

/// The i-th tab, from the strip metrics of the last drawn frame. Draw
/// and hit-test share these numbers through [`Shell::strip`] rather than
/// through arithmetic on the window height: the strip's dimensions are
/// theme tokens now, and input arrives with no drawing context to read
/// them through. Before the first frame the metrics are zero, the rects
/// are empty and a click falls through — there is nothing on screen for
/// it to have hit.
fn tab_rect(r: RectC, pad: f32, tab_h: f32, gap: f32, count: u32, i: u32) -> RectC {
    let count = count.max(1) as f32;
    let tw = (r.w - 2.0 * pad - gap * (count - 1.0)) / count;
    RectC { x: r.x + pad + (tw + gap) * i as f32, y: r.y + pad, w: tw, h: tab_h }
}

/// A frame with its corners cut off — the toolkit's `chamfer_frame`,
/// which is a closed polyline through eight points and nothing more.
/// Since the host took the panel's container this draws only the SCROLL
/// pill's ring, when `badge.corner` cuts one.
fn chamfer(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, t: f32, c: ColorC) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let pts: [f32; 16] = [
        x + cut, y,
        x + w - cut, y,
        x + w, y + cut,
        x + w, y + h - cut,
        x + w - cut, y + h,
        x + cut, y + h,
        x, y + h - cut,
        x, y + cut,
    ];
    (api.polyline)(ctx, pts.as_ptr(), 8, t, c, true);
}

/// The filled version — the toolkit's `chamfer_fill`, as three quads:
/// the middle band and the two trapezoids the cut corners leave.
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

pub struct Shell {
    /// Grown once and reused; never shrunk, because the frame after a
    /// resize would only grow it again.
    cells: Vec<CellC>,
    /// What the last frame settled on. The host asks for this after the
    /// draw loop, with no context in scope to recompute it from.
    grid: (u32, u32),
    /// How many tabs the strip has. Cached because a click arrives
    /// without the host data that carries it.
    tab_count: u32,
    /// The resolved token table, rebuilt whenever the theme epoch moves.
    tokens: Option<Tokens>,
    /// Strip metrics (pad, tab height, gap) in device px, kept from the
    /// last draw for the input handlers — see [`tab_rect`].
    strip: (f32, f32, f32),
    /// `terminal.wheel_lines`, kept from the last draw for the same
    /// reason.
    wheel_lines: f32,
    /// Where the cell grid sat in the last drawn frame — origin and
    /// cell size in device px — and how many cells were delivered.
    /// What `drag` maps pixels against: input arrives with no drawing
    /// context, exactly like the tab strip's `strip`.
    sel_grid: (f32, f32, f32, f32),
    sel_dims: (u32, u32),
    /// The line id of that frame's first view row, echoed back in
    /// every TermSelect so the host resolves rows against the view
    /// this widget actually drew (the drag-vs-feed race, F1 §2.7).
    sel_base: (u32, u32),
    /// The TermSelect payload handed to the host — owned by the
    /// instance until its next call, the `last_path` discipline.
    select_out: TermSelectC,
}

impl Shell {
    fn new() -> Shell {
        Shell {
            cells: Vec::new(),
            grid: (0, 0),
            tab_count: 0,
            tokens: None,
            strip: (0.0, 0.0, 0.0),
            wheel_lines: 0.0,
            sel_grid: (0.0, 0.0, 0.0, 0.0),
            sel_dims: (0, 0),
            sel_base: (0, 0),
            select_out: TermSelectC {
                op: 0,
                kind: 0,
                col: 0,
                row: 0,
                base_lo: 0,
                base_hi: 0,
            },
        }
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, host_data: *const c_void, r: RectC) {
        // The theme entries exist from ABI 5 up. Linked statically this
        // is always true; attached to an older dlopen host it is not,
        // and then every read below answers the engine's raw defaults —
        // the same values a current host gives for an undeclared token.
        let themed = api.abi_version >= THEME_ABI;
        let ids = if themed {
            let epoch = (api.theme_epoch)(ctx);
            match self.tokens {
                Some(t) if t.epoch == epoch => t,
                _ => *self.tokens.insert(Tokens::resolve(api, ctx)),
            }
        } else {
            Tokens::default()
        };
        let px = |id: u32| if themed { (api.theme_px)(ctx, id) } else { 0.0 };
        let ink = |id: u32| if themed { (api.theme_color)(ctx, id) } else { RAW_INK };
        // A colour used as a BED — the pill's interior. Missing, it
        // answers the engine's raw near-black rather than the mid grey.
        let bed = |id: u32| {
            if themed {
                (api.theme_bed)(ctx, id)
            } else {
                ColorC { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
            }
        };
        let flag = |id: u32| themed && (api.theme_flag)(ctx, id) != 0;
        let word = |id: u32| if themed { (api.theme_enum)(ctx, id) } else { 0 };
        let style = |state: u32| {
            let mut out = RAW_STATE;
            if themed {
                (api.theme_class_state)(
                    ctx,
                    ids.tab_class,
                    state,
                    &mut out,
                    std::mem::size_of::<StateStyleC>() as u32,
                );
            }
            out
        };

        let pad = px(ids.tab_pad);
        let tab_h = px(ids.tab_h);
        let rule_gap = px(ids.rule_gap);
        let term_pad = px(ids.term_pad);

        // The grid area, which is what the host divides into cells and
        // what the user's PTY ends up sized to. Its top clears the strip
        // by tab.rule_gap; the other three sides keep terminal.pad.
        let grid_r = RectC {
            x: r.x + term_pad,
            y: r.y + pad + tab_h + rule_gap,
            w: r.w - 2.0 * term_pad,
            h: r.h - (pad + tab_h + rule_gap) - term_pad,
        };

        let stride = std::mem::size_of::<CellC>() as u32;
        let mut view = TermViewC::empty();
        let mut req = TermReqC::empty();
        req.area = grid_r;
        req.cell_stride = stride;
        for _ in 0..2 {
            req.cells = self.cells.as_mut_ptr();
            req.cells_bytes = (self.cells.len() as u32).saturating_mul(stride);
            (api.term_view)(
                host_data,
                ctx,
                &req,
                std::mem::size_of::<TermReqC>() as u32,
                &mut view,
                std::mem::size_of::<TermViewC>() as u32,
            );
            if view.flags & VIEW_TRUNCATED == 0 {
                break;
            }
            // Only ever on the frame a resize lands, and then not again.
            let want = (view.cols as usize).saturating_mul(view.rows as usize);
            self.cells.resize(want.max(1), CellC::absent());
        }

        if view.tab_count > 0 {
            self.tab_count = view.tab_count;
        } else if self.tab_count == 0 {
            // Before the host has said: the declared strip size, so a
            // click that beats the first answer lands where the tabs
            // will be.
            self.tab_count = px(ids.tab_count) as u32;
        }
        // No terminal means no frame and no tabs at all, and the grid is
        // left alone rather than reported as zero.
        if view.flags & VIEW_LIVE == 0 {
            return;
        }
        self.grid = (view.cols, view.rows);

        let gap = px(ids.tab_gap);
        // What the input handlers replay against: input runs with no
        // drawing context, so the strip's metrics, the wheel step and
        // the grid's geometry travel through the instance from the
        // frame that drew them.
        self.strip = (pad, tab_h, gap);
        self.wheel_lines = px(ids.wheel_lines);
        self.sel_grid = (grid_r.x, grid_r.y, view.cell_w, view.cell_h);
        self.sel_dims = (view.view_cols, view.view_rows);
        self.sel_base = (view.first_id_lo, view.first_id_hi);

        // No frame here: the container is the host's (u2 §4), already on
        // screen by the time this runs, and `r` is the content box it left.

        // --- tab strip ------------------------------------------------
        let (mut mx, mut my) = (0.0f32, 0.0f32);
        (api.mouse)(ctx, &mut mx, &mut my);
        let label_px = px(ids.label_size).max(px(ids.label_min));
        let track = label_px * px(ids.label_track);
        let lead = px(ids.label_lead);
        let case = word(ids.label_case);
        // Optical centring nudges an upper-case run by a fraction of its
        // px. `rhythm.center_mode` declares `optical | geometric`
        // (optical = 0), and it applies when the run's case transform is
        // upper (1) or smallcaps (3) — the master's own rule for it.
        let bias = if word(ids.center_mode) == 0 && matches!(case, 1 | 3) {
            px(ids.center_bias) * label_px
        } else {
            0.0
        };
        let skew = px(ids.tab_skew);
        let count = self.tab_count.max(1);
        for i in 0..count {
            let tr = tab_rect(r, pad, tab_h, gap, count, i);
            let occupied = view.tabs & (1u32 << i.min(31)) != 0;
            let is_active = i == view.tab_active;
            let hover = contains(&tr, mx, my);
            let st = style(if is_active {
                if hover { ST_SELECTED_HOVER } else { ST_SELECTED }
            } else if hover {
                ST_HOVER
            } else if occupied {
                ST_IDLE
            } else {
                ST_DISABLED
            });
            let right = tr.x + tr.w;
            let bottom = tr.y + tr.h;
            // A sheared tab is a quad; without shear it is the family's
            // shape — the same corners the frames and every other
            // button wear, drawn by the host so this plugin never has
            // to know how an arc is tessellated.
            let ring = skew <= 0.0 && api.has_ring();
            let (cs, radius) = if ring {
                let cs = match enum_word(api, ctx, ids.tab_corner_style).as_str() {
                    "round" => CORNER_ROUND,
                    "chamfer" => CORNER_CHAMFER,
                    _ => CORNER_SQUARE,
                };
                (cs, px(ids.tab_corner))
            } else {
                (CORNER_SQUARE, 0.0)
            };
            let pts: [f32; 8] = [
                tr.x + skew, tr.y,
                right, tr.y,
                right - skew, bottom,
                tr.x, bottom,
            ];
            if ring {
                let rc = RectC { x: tr.x, y: tr.y, w: tr.w, h: tr.h };
                (api.ring_fill)(ctx, rc, cs, radius, st.fill);
                // The rung's ring, drawn at last: this is where a
                // theme's selected.edge = @border.focus reaches the
                // active tab (u2 §2.9) — the whole ladder, not a
                // special case here.
                if st.edge_width > 0.0 && st.edge.a > 0.0 {
                    (api.ring)(ctx, rc, cs, radius, st.edge_width, st.edge);
                }
            } else {
                (api.quad)(ctx, pts.as_ptr(), st.fill);
                if st.edge_width > 0.0 && st.edge.a > 0.0 {
                    (api.polyline)(ctx, pts.as_ptr(), 4, st.edge_width, st.edge, true);
                }
            }

            let label = if occupied {
                if i == 0 { "Main shell" } else { "Shell" }
            } else {
                "Empty"
            };
            let text = if is_active {
                format!("#{} {}", i + 1, label)
            } else if !occupied && hover {
                format!("+ {label}")
            } else {
                label.to_string()
            };
            draw_text(
                api,
                ctx,
                FONT_UI,
                label_px,
                tr.x + tr.w / 2.0,
                tr.y + (tr.h - label_px * lead) / 2.0 + bias,
                &recase(case, text),
                st.text,
                track,
                1,
            );
        }
        let strip_l = tab_rect(r, pad, tab_h, gap, count, 0);
        let strip_r = tab_rect(r, pad, tab_h, gap, count, count - 1);
        let rule_w = px(ids.rule_w);
        if rule_w > 0.0 {
            let under = strip_l.y + strip_l.h + rule_gap;
            (api.line)(
                ctx,
                strip_l.x,
                under,
                strip_r.x + strip_r.w,
                under,
                rule_w,
                ink(ids.rule_c),
            );
        }

        // --- the grid -------------------------------------------------
        //
        // Strictly in order per cell — background, glyph, underline —
        // and nothing hoisted across cells. A glyph rounds to its own
        // bitmap and a box-drawing character can overhang its cell, so
        // drawing all the backgrounds first would change which primitive
        // ends up on top. The cells arrive with their colours already
        // final; only the underline's geometry is the theme's business
        // here.
        let ul_h = px(ids.cell_ul_h);
        let ul_gap = px(ids.cell_ul_gap);
        let sel_pad = px(ids.sel_pad);
        let (cw, ch_h) = (view.cell_w, view.cell_h);
        let ncells = (view.view_rows as usize).saturating_mul(view.view_cols as usize);
        let cells = &self.cells[..ncells.min(self.cells.len())];
        for y in 0..view.view_rows as usize {
            let cy = grid_r.y + y as f32 * ch_h;
            for x in 0..view.view_cols as usize {
                let Some(cell) = cells.get(y * view.view_cols as usize + x) else {
                    break;
                };
                // The host sends the FLAG, never a baked colour: the
                // invert mode below needs the original colours, and a
                // wash baked into `bg` could not be taken apart again.
                let selected =
                    cell.flags & CELL_SELECTED != 0 && cell.flags & CELL_ABSENT == 0;
                // Nothing to draw: the second half of a wide character,
                // or a position no cell exists at. A SELECTED spacer
                // still gets its column of wash, so a wide character's
                // selection has no seam.
                if cell.width == 0 && !selected {
                    continue;
                }
                let cx = grid_r.x + x as f32 * cw;
                if cell.flags & CELL_HAS_BG != 0 && cell.width > 0 {
                    let w = (cw * cell.width as f32).min(grid_r.x + grid_r.w - cx);
                    (api.rect)(ctx, RectC { x: cx, y: cy, w, h: ch_h }, cell.bg);
                }
                if selected {
                    // One column per flagged cell; the bleed is the
                    // theme's terminal.selection_pad (0 by default, so
                    // the wash follows the cell grid exactly). Tint
                    // washes term.selection over the cell; invert lays
                    // the glyph's own colour down as the bed.
                    let wash = RectC {
                        x: cx - sel_pad,
                        y: cy - sel_pad,
                        w: cw + 2.0 * sel_pad,
                        h: ch_h + 2.0 * sel_pad,
                    };
                    let bed = if ids.sel_tint { ink(ids.sel_wash) } else { cell.fg };
                    (api.rect)(ctx, wash, bed);
                }
                // The glyph's ink: its own outside a selection; inside
                // one, selection_fg when tinting, or the cell's own
                // background (term.bg when it had none) when inverting.
                let glyph_c = if !selected {
                    cell.fg
                } else if ids.sel_tint {
                    ink(ids.sel_fg)
                } else if cell.flags & CELL_HAS_BG != 0 {
                    cell.bg
                } else {
                    ink(ids.term_bg)
                };
                if cell.width > 0 && cell.ch != b' ' as u32 {
                    glyph(api, ctx, cell.ch, cell.font, cx, cy, view.px, glyph_c);
                }
                if cell.width > 0 && cell.flags & CELL_UNDERLINE != 0 && ul_h > 0.0 {
                    // One cell wide even under a double-width character,
                    // as it has always been — in the glyph's current
                    // ink, so an inverted selection keeps it visible.
                    (api.rect)(
                        ctx,
                        RectC { x: cx, y: cy + ch_h - ul_gap - ul_h, w: cw, h: ul_h },
                        glyph_c,
                    );
                }
            }
        }

        // --- cursor ---------------------------------------------------
        if view.flags & VIEW_CURSOR != 0 {
            // The blink is the theme's, period in ms and duty 0..1.
            // Disabled — or missing, or a nonsense period — freezes the
            // cursor ON, never off: the motion is decoration, and the
            // cursor's visibility is not.
            let period = px(ids.blink_period) as f64;
            let shown = if flag(ids.blink_on) && period > 0.0 {
                let t = (api.elapsed)(ctx) * 1000.0;
                (t % period) / period < px(ids.blink_duty) as f64
            } else {
                true
            };
            if shown {
                let cx = grid_r.x + view.cursor_col as f32 * cw;
                let cy = grid_r.y + view.cursor_row as f32 * ch_h;
                // Deliberately not clipped to the grid: the block goes
                // wherever the cursor is, which is what it has always
                // done and what makes a cursor past the last column
                // visible rather than silently absent. Its colours are
                // the host's, resolved into the view like every cell's.
                (api.rect)(ctx, RectC { x: cx, y: cy, w: cw, h: ch_h }, view.cursor_bg);
                if view.cursor_ch != b' ' as u32 {
                    glyph(
                        api,
                        ctx,
                        view.cursor_ch,
                        nacelle::font::FONT_MONO,
                        cx,
                        cy,
                        view.px,
                        view.cursor_fg,
                    );
                }
            }
        }

        // --- scrollback indicator -------------------------------------
        //
        // A badge with severity.info (u2 §2.9): the same string as ever,
        // in the corner it has always held, now inside the status pill of
        // images 1, 3 and 4. Its arrangement is the severity's own
        // badge_style WORD (ABI 6) — the hollow the master declares for
        // info (ring and text in the severity's colours over its fill
        // bed), or the solid a theme may say instead — no longer this
        // file's guess; the indices alone could not tell the styles
        // apart.
        if view.view_offset > 0 {
            let px_s = px(ids.badge_size).max(px(ids.badge_min));
            let lead = px(ids.badge_lead).max(1.0);
            let track = px_s * px(ids.badge_track);
            let inset = px(ids.ind_inset);
            let text = recase(
                word(ids.badge_case),
                format!("Scroll +{}", view.view_offset),
            );
            let tw = (api.measure)(ctx, FONT_UI, px_s, text.as_ptr(), text.len() as u32, track);
            let w = tw + 2.0 * px(ids.badge_pad);
            let h = px(ids.badge_h).max(1.0);
            let pill = RectC {
                x: grid_r.x + grid_r.w - inset - w,
                y: grid_r.y + inset,
                w,
                h,
            };
            let cut = px(ids.badge_corner);
            let border = px(ids.badge_border);
            let (fill, text_c) = if ids.pill_solid {
                (ink(ids.info_text), ink(ids.info_on))
            } else {
                (bed(ids.info_fill), ink(ids.info_text))
            };
            if cut > 0.0 {
                chamfer_fill(api, ctx, pill, cut.min(h / 2.0), fill);
                if !ids.pill_solid && border > 0.0 {
                    chamfer(api, ctx, pill, cut.min(h / 2.0), border, ink(ids.info_edge));
                }
            } else {
                // `pill` is a negative sentinel until R5 lands: square it is.
                (api.rect)(ctx, pill, fill);
                if !ids.pill_solid && border > 0.0 {
                    (api.rect_outline)(ctx, pill, border, ink(ids.info_edge));
                }
            }
            draw_text(
                api,
                ctx,
                FONT_UI,
                px_s,
                pill.x + pill.w / 2.0,
                pill.y + (pill.h - px_s * lead) / 2.0,
                &text,
                text_c,
                track,
                1,
            );
        }
    }
}

// ----------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Shell::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Shell) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Shell> {
    unsafe { (instance as *mut Shell).as_mut() }
}

extern "C" fn draw_c(
    instance: *mut c_void,
    ctx: *mut c_void,
    host_data: *const c_void,
    r: RectC,
) {
    let (Some(api), Some(this)) = (host(), state(instance)) else { return };
    // A panic reaching extern "C" ends the process. Caught here it costs
    // a half-drawn frame, which is why this crate must keep unwinding.
    let _ = catch_unwind(AssertUnwindSafe(|| this.draw(api, ctx, host_data, r)));
}

extern "C" fn click_c(
    instance: *mut c_void,
    x: f32,
    y: f32,
    r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return;
    };
    let (pad, tab_h, gap) = this.strip;
    let count = this.tab_count.max(1);
    for i in 0..count {
        if contains(&tab_rect(r, pad, tab_h, gap, count, i), x, y) {
            out.kind = ACTION_SELECT_TAB;
            out.index = i;
            return;
        }
    }
    out.kind = ACTION_NONE;
}

extern "C" fn wheel_c(
    instance: *mut c_void,
    dy: f32,
    _r: RectC,
    _win_w: f32,
    _win_h: f32,
    out: *mut ActionC,
) {
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return;
    };
    out.kind = ACTION_SCROLL_TERMINAL;
    // A saturating cast: NaN becomes 0 rather than something wild.
    out.lines = (dy * this.wheel_lines) as i32;
}

/// A drag over the panel, mapped to cell coordinates — this widget
/// alone knows its cell metrics and tab-strip offset, so the pixel→cell
/// translation happens HERE and the host receives cells (F1 §2.4). The
/// echoed base ties the cells to the frame they were measured against.
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
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return;
    };
    out.kind = ACTION_NONE;
    let (gx, gy, cw, ch) = this.sel_grid;
    let (cols, rows) = this.sel_dims;
    if cw <= 0.0 || ch <= 0.0 || cols == 0 || rows == 0 {
        return; // nothing drawn yet — nothing to select
    }
    // A press outside the cell grid is not a selection: declining the
    // Begin hands the press back to the host's click machinery, which
    // is how the tab strip keeps its clicks. Move and End clamp
    // instead — a drag that leaves the grid selects to its edge.
    if phase == DRAG_BEGIN
        && !(x >= gx && x < gx + cw * cols as f32 && y >= gy && y < gy + ch * rows as f32)
    {
        return;
    }
    let col = (((x - gx) / cw).floor() as i64).clamp(0, cols as i64 - 1) as u32;
    let row = (((y - gy) / ch).floor() as i64).clamp(0, rows as i64 - 1) as u32;
    this.select_out = TermSelectC {
        op: match phase {
            DRAG_BEGIN => SELECT_OP_BEGIN,
            DRAG_END => SELECT_OP_END,
            _ => SELECT_OP_EXTEND,
        },
        // Always Cells: the HOST owns the double/triple-click kinds —
        // a widget cannot see click counts.
        kind: SELECT_KIND_CELLS,
        col,
        row,
        base_lo: this.sel_base.0,
        base_hi: this.sel_base.1,
    };
    out.kind = ACTION_TERM_SELECT;
    out.data = &this.select_out as *const TermSelectC as *const u8;
    out.data_len = std::mem::size_of::<TermSelectC>() as u32;
}

extern "C" fn grid_c(instance: *mut c_void, cols: *mut u32, rows: *mut u32) {
    let Some(this) = (unsafe { (instance as *const Shell).as_ref() }) else { return };
    unsafe {
        if !cols.is_null() {
            *cols = this.grid.0;
        }
        if !rows.is_null() {
            *rows = this.grid.1;
        }
    }
}

extern "C" fn key_feedback_c(_: *mut c_void, _: u32, _: *const u8, _: u32) {}

/// Character cells. A bigger panel is more of them, not larger
/// ones — their size comes from the terminal font setting. The tab
/// strip above them keeps to the reference box.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_REFERENCE
}

/// No title band: the terminal's heading is its tab strip, which stays
/// the widget's own (u2 §2.9).
extern "C" fn chrome_c(
    _: *mut c_void,
    _: *mut c_void,
    _: *const c_void,
    _: *mut ChromeC,
    _: u32,
) -> u32 {
    0
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

/// # Safety
/// Called by the host with its own interface, once, before anything else.
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

#[cfg(feature = "dyn")]
#[no_mangle]
pub unsafe extern "C" fn nacelle_plugin_attach(api: *const HostApi) -> *const PluginApi {
    if !nacelle::runtime::attach(api) {
        return std::ptr::null();
    }
    HOST = api.as_ref();
    &API
}
