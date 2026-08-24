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

use nacelle::plugin_shapes;
use nacelle::runtime::{
    ActionC, CellC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, TermReqC,
    TermSelectC, TermViewC, ABI_VERSION, ACTION_NONE, ACTION_SCROLL_TERMINAL,
    ACTION_SELECT_TAB, ACTION_TERM_SELECT, CELL_ABSENT, CELL_HAS_BG, CELL_SELECTED,
    CORNER_CHAMFER, CORNER_SQUARE,
    CELL_UNDERLINE, DRAG_BEGIN, DRAG_END, SELECT_KIND_CELLS, SELECT_OP_BEGIN, SELECT_OP_END,
    SELECT_OP_EXTEND, VIEW_CURSOR, VIEW_LIVE, VIEW_TRUNCATED,
};
use nacelle::widget::factory::BuiltinWidget;
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
// engine's raw default for its KIND — 0.0 for a length, and on a host
// that cannot be asked at all, no ink — so the widget draws less rather
// than drawing a value that used to be the design.

/// No ink at all: what every colour becomes on a host too old to carry
/// the theme entries.
///
/// Not a grey and not a black. A colour chosen where the theme cannot be
/// reached is a design decision taken in the dark, and this program has
/// none of those — so the strip and the pill draw NOTHING instead, which
/// is the same clean bail `ai` takes for the same host.
const NO_INK: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// The raw rung for that host: no ink and no width, so every shape
/// guarded by `edge_width > 0.0 && edge.a > 0.0` stays undrawn.
const RAW_STATE: StateStyleC = StateStyleC {
    fill: NO_INK,
    edge: NO_INK,
    text: NO_INK,
    glyph: NO_INK,
    edge_width: 0.0,
    glow_radius: 0.0,
    glow_alpha: 0.0,
    elevation: 0.0,
};

/// Rows of the class x state matrix, in the ladder's declaration order.
/// The three this widget never enters (press, dragging, DISABLED) have
/// no index here on purpose — a tab is selected, hovered or resting, and
/// that is the whole ladder it wears. Disabled is the one worth spelling
/// out, because this file used to reach for it: see [`tab_face`].
const ST_IDLE: u32 = 0;
const ST_HOVER: u32 = 1;
const ST_SELECTED: u32 = 3;
const ST_SELECTED_HOVER: u32 = 4;

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
    // type.<tab.role>.* — the role the master BINDS a tab's label to,
    // followed as a word rather than spelled out here. The two used to
    // agree by coincidence (`tab.role = button`); re-roling the strip in
    // a theme moved nothing, because the binding existed only in the
    // master.
    label_size: u32,
    label_min: u32,
    label_track: u32, // in em
    label_lead: u32,
    label_case: u32,
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
    // the cursor's SHAPE and the two thin carets' lengths. The shape is
    // `term.cursor.style`'s WORD; the lengths sit in `[terminal]`, the
    // section that holds the cell grid's geometry, one section away.
    cur_ul_h: u32,   // terminal.cursor.underline_h — the CARET's rule
    cur_ul_gap: u32, // terminal.cursor.underline_gap
    cur_bar_w: u32,  // terminal.cursor.bar_w
    // The row's own multiplier, read here for one reason: the host
    // measured the CELL with it, and everything drawn inside the cell
    // has to know how much of that cell is air (see `half_leading`).
    line_height: u32, // terminal.line_height
    // the SCROLL +n readout — a status pill in a corner, which is
    // precisely the images' badge (u2 §2.9): severity.info's colours,
    // the badge role's type, the badge component's box
    ind_inset: u32, // terminal.indicator.inset
    // type.<terminal.indicator.role>.* — the readout's own binding, and
    // NOT `badge.role`: the pill borrows the badge component's box
    // (height, padding, corner, ring) because it is one, but which type
    // a scroll readout is set in is the terminal's to say, and the
    // master says `caption` where this file drew `badge`.
    ind_size: u32,
    ind_min: u32,
    ind_track: u32, // in em
    ind_lead: u32,
    ind_case: u32,
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
    /// The caret's shape — `term.cursor.style`'s WORD.
    cursor_style: CursorStyle,
}

/// The shape the caret takes, which is `term.cursor.style`'s WORD.
///
/// Until 2026-08-17 this widget drew a full block and nothing else, so
/// the token, and the three lengths `[terminal]` keeps for the other two
/// shapes, had no reader at all (audit 2026-08-17, Z17).
///
/// `block` is where an unknown word lands: it is the master's own
/// default, it is the shape every frame before this one drew, and a
/// caret whose shape a theme misspelled is still a caret — the person
/// typing is looking straight at it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum CursorStyle {
    /// The whole cell, with the glyph under it redrawn in the caret's
    /// own ink so it survives being covered.
    #[default]
    Block,
    /// A rule down the leading edge of the cell, `terminal.cursor.bar_w`
    /// wide. The WORD and the length agree — `bar` and `bar_w` — and so
    /// does `field.caret_style`, which is the other half of the master
    /// that names this shape.
    Bar,
    /// A rule under the glyph, `terminal.cursor.underline_h` thick and
    /// `terminal.cursor.underline_gap` below the floor of the glyph's
    /// line box — the caret's own pair, not the cell underline's
    /// `cell_underline_*` that SGR 4 draws.
    Underline,
}

impl CursorStyle {
    fn of(word: &str) -> CursorStyle {
        match word {
            "bar" => CursorStyle::Bar,
            "underline" => CursorStyle::Underline,
            _ => CursorStyle::Block,
        }
    }
}

/// The lengths the two thin carets are cut from, resolved for the frame.
#[derive(Clone, Copy, Default, Debug)]
struct CursorGeom {
    /// `terminal.cursor.underline_h`
    underline_h: f32,
    /// `terminal.cursor.underline_gap`
    underline_gap: f32,
    /// `terminal.cursor.bar_w`
    bar_w: f32,
    /// Half the row's air — [`half_leading`]. The underline caret hangs
    /// off the GLYPH, so it has to know where inside the cell the glyph
    /// was put.
    lead: f32,
}

/// How much of a row is air, on ONE side of the glyph.
///
/// `terminal.line_height` multiplies the face's own line box to make the
/// cell (libnacelle `term::Grid::measure`), so dividing the cell by that
/// same token gives the line box back, and what is left over is air. It
/// is shared above and below: that is what keeps the glyph in the middle
/// of an opened-up row instead of hanging from its ceiling, and what
/// keeps both underlines under the character rather than under the row.
///
/// The host measures the cell and this widget draws inside it, so this
/// division is the one seam where the two have to agree about the same
/// token. It belongs beside `cell_h` in the view struct, and can go
/// there when the ABI next grows; until then it is read on both sides of
/// the boundary, off one theme at one epoch.
///
/// Anything at or under 1.0 — a squeezed row, a missing token, a theme
/// with a nonsense value — is no air at all. There is nothing to share,
/// and a glyph lifted out of a short row would land in the row above.
fn half_leading(cell_h: f32, line_height: f32) -> f32 {
    // Written as a refused comparison rather than `<= 1.0` so that a NaN
    // answers zero instead of falling through, and with max/min rather
    // than clamp for the same reason: clamp panics on a NaN bound.
    if !(line_height > 1.0) {
        return 0.0;
    }
    ((cell_h - cell_h / line_height) * 0.5).max(0.0).min(cell_h * 0.5)
}

/// The top edge of an underline: `gap` under the floor of the glyph's
/// LINE BOX, which is the cell's own floor while `terminal.line_height`
/// is 1.00 and `lead` above it once a row has been opened up.
///
/// One function for two callers on purpose. SGR 4's cell underline and
/// the underline caret are drawn from different tokens by different
/// halves of this file, and a person looking at an underlined character
/// with the cursor on it has to see one rule, not two.
fn underline_y(cell_y: f32, cell_h: f32, lead: f32, gap: f32, h: f32) -> f32 {
    cell_y + cell_h - lead - gap - h
}

/// The caret's quad inside its cell, and whether that quad COVERS the
/// glyph underneath.
///
/// The second half of the answer is the whole difference between the
/// shapes: a block hides the character it sits on, so the character has
/// to be laid back over it in the caret's ink, while a beam and an
/// underline leave the grid's own glyph showing and must not draw a
/// second copy of it.
///
/// Both thin shapes are floored at one device pixel, and that is
/// arithmetic rather than a look: a theme that asked for a beam asked
/// for a caret, and a rule of no width is not one. It is the same rule
/// the blink already follows — the motion is decoration, the caret's
/// visibility is not.
fn cursor_quad(style: CursorStyle, cell: RectC, g: CursorGeom) -> (RectC, bool) {
    match style {
        CursorStyle::Block => (cell, true),
        CursorStyle::Bar => (RectC { w: g.bar_w.max(1.0).min(cell.w), ..cell }, false),
        CursorStyle::Underline => {
            let h = g.underline_h.max(1.0).min(cell.h);
            // Through the same function the cell's SGR-4 underline goes
            // through, so a caret and an underlined cell sit on one line
            // at every line height.
            let y = underline_y(cell.y, cell.h, g.lead, g.underline_gap, h);
            (RectC { y, h, ..cell }, false)
        }
    }
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

/// The name of one token of the role a `*_role` binding names, or `None`
/// for a master that binds no role — which leaves every id MISSING, and
/// type of no size draws nothing. Substituting a role here would be this
/// file deciding how the interface is set.
fn role_token(role: &str, suffix: &str) -> Option<String> {
    if role.is_empty() {
        return None;
    }
    Some(format!("type.{role}.{suffix}"))
}

impl Tokens {
    fn resolve(api: &HostApi, ctx: *mut c_void) -> Tokens {
        let t = |n: &str| (api.theme_token)(n.as_ptr(), n.len() as u32);
        let c = |n: &str| (api.theme_class)(n.as_ptr(), n.len() as u32);
        let style = enum_word(api, ctx, t("severity.info.badge_style"));
        let sel_mode = enum_word(api, ctx, t("term.selection.mode"));
        let cursor_style = enum_word(api, ctx, t("term.cursor.style"));
        // The two type bindings, followed to the roles they name.
        let tab_role = enum_word(api, ctx, t("tab.role"));
        let ind_role = enum_word(api, ctx, t("terminal.indicator.role"));
        let of = |role: &str, suffix: &str| match role_token(role, suffix) {
            Some(name) => t(&name),
            None => u32::MAX,
        };
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
            label_size: of(&tab_role, "size"),
            label_min: of(&tab_role, "min_px"),
            label_track: of(&tab_role, "tracking"),
            label_lead: of(&tab_role, "leading"),
            label_case: of(&tab_role, "case"),
            center_mode: t("rhythm.center_mode"),
            center_bias: t("rhythm.cap_center_bias"),
            term_pad: t("terminal.pad"),
            cell_ul_h: t("terminal.cell_underline_h"),
            cell_ul_gap: t("terminal.cell_underline_gap"),
            wheel_lines: t("terminal.wheel_lines"),
            blink_on: t("motion.term_cursor_blink.enabled"),
            blink_period: t("motion.term_cursor_blink.period_ms"),
            blink_duty: t("motion.term_cursor_blink.duty"),
            cur_ul_h: t("terminal.cursor.underline_h"),
            cur_ul_gap: t("terminal.cursor.underline_gap"),
            cur_bar_w: t("terminal.cursor.bar_w"),
            line_height: t("terminal.line_height"),
            ind_inset: t("terminal.indicator.inset"),
            ind_size: of(&ind_role, "size"),
            ind_min: of(&ind_role, "min_px"),
            ind_track: of(&ind_role, "tracking"),
            ind_lead: of(&ind_role, "leading"),
            ind_case: of(&ind_role, "case"),
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
            cursor_style: CursorStyle::of(&cursor_style),
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

/// What a tab wears — the rung of the state ladder and the words on it.
///
/// One function for two answers because after 2026-08-18 they are one
/// decision: an empty slot is told apart from an occupied one by its
/// WORDS, and by nothing on the ladder at all.
///
/// # Why empty is not disabled
///
/// It was, and only while standing still: the rung went disabled at rest
/// and hover under the pointer, so an empty slot lit up — the one thing
/// 5.21 states cannot happen ("Disabled is first on purpose: a disabled
/// control must never light up under the pointer"). Two ways out, and
/// they are decided by what the slot IS to the person looking at it.
///
/// It is an invitation. A click on an empty slot does not fail and does
/// not beep: the host reads the tab index, finds no session behind it
/// and OPENS one, in the directory the active shell is standing in.
/// `pointer` already answers "hand" over the whole strip, and this file
/// already writes `+ Empty` under the cursor. A control that acts on a
/// click is not a disabled control, so the lighting is right and the
/// name was wrong — the empty slot rests on `idle` like every other tab
/// and keeps saying what it is in the only place a difference of KIND
/// belongs, its label.
///
/// The other branch — keep it disabled, drop the hover — would have to
/// keep the click too (nothing else opens a session) and would leave a
/// control that looks dead, gives no answer to the pointer, and works.
/// That is a worse lie than the one this replaces.
fn tab_face(occupied: bool, is_active: bool, hover: bool, index: u32) -> (u32, String) {
    let rung = match (is_active, hover) {
        (true, true) => ST_SELECTED_HOVER,
        (true, false) => ST_SELECTED,
        (false, true) => ST_HOVER,
        (false, false) => ST_IDLE,
    };
    let label = if occupied {
        if index == 0 { "Main shell" } else { "Shell" }
    } else {
        "Empty"
    };
    let text = if is_active {
        format!("#{} {}", index + 1, label)
    } else if !occupied && hover {
        format!("+ {label}")
    } else {
        label.to_string()
    };
    (rung, text)
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
        let ink = |id: u32| if themed { (api.theme_color)(ctx, id) } else { NO_INK };
        // A colour used as a BED — the pill's interior. A host that
        // cannot be asked answers no colour at all: a bed of some
        // chosen black is this file painting over the terminal in a
        // shade nobody picked, and a pill of no ink is simply not there.
        let bed = |id: u32| if themed { (api.theme_bed)(ctx, id) } else { NO_INK };
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
            // The rung and the words together — one answer, because the
            // difference between an empty slot and a full one lives in
            // the words alone (see `tab_face`).
            let (rung, text) = tab_face(occupied, is_active, hover, i);
            let st = style(rung);
            let right = tr.x + tr.w;
            let bottom = tr.y + tr.h;
            // A sheared tab is a quad; without shear it is the family's
            // shape — the same corners the frames and every other
            // button wear, drawn by the host so this plugin never has
            // to know how an arc is tessellated.
            let ring = skew <= 0.0 && api.has_ring();
            let (cs, radius) = if ring {
                let cs = nacelle::corner::code_of(&enum_word(api, ctx, ids.tab_corner_style));
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
        // How far into its cell the glyph's line box starts. Zero at the
        // master's line height of 1.00, where the cell IS the line box —
        // so the ordinary picture is untouched — and half the air above
        // the glyph once a theme opens the rows up.
        let row_lead = half_leading(ch_h, px(ids.line_height));
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
                    glyph(api, ctx, cell.ch, cell.font, cx, cy + row_lead, view.px, glyph_c);
                }
                if cell.width > 0 && cell.flags & CELL_UNDERLINE != 0 && ul_h > 0.0 {
                    // One cell wide even under a double-width character,
                    // as it has always been — in the glyph's current
                    // ink, so an inverted selection keeps it visible.
                    // Under the GLYPH, not under the row: with air in
                    // the row the two are not the same line.
                    (api.rect)(
                        ctx,
                        RectC {
                            x: cx,
                            y: underline_y(cy, ch_h, row_lead, ul_gap, ul_h),
                            w: cw,
                            h: ul_h,
                        },
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
                // Deliberately not clipped to the grid: the caret goes
                // wherever the cursor is, which is what it has always
                // done and what makes a cursor past the last column
                // visible rather than silently absent. Its colours are
                // the host's, resolved into the view like every cell's;
                // its SHAPE is the theme's `term.cursor.style`.
                let (quad, covers) = cursor_quad(
                    ids.cursor_style,
                    RectC { x: cx, y: cy, w: cw, h: ch_h },
                    CursorGeom {
                        underline_h: px(ids.cur_ul_h),
                        underline_gap: px(ids.cur_ul_gap),
                        bar_w: px(ids.cur_bar_w),
                        lead: row_lead,
                    },
                );
                (api.rect)(ctx, quad, view.cursor_bg);
                // Only a block hid the character it sits on, so only a
                // block owes it back — and it owes it in the place the
                // grid loop drew it, air and all. Laying it over a bar
                // would paint a second copy of a glyph that is already
                // there, in the wrong ink.
                if covers && view.cursor_ch != b' ' as u32 {
                    glyph(
                        api,
                        ctx,
                        view.cursor_ch,
                        nacelle::font::FONT_MONO,
                        cx,
                        cy + row_lead,
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
            let px_s = px(ids.ind_size).max(px(ids.ind_min));
            let lead = px(ids.ind_lead).max(1.0);
            let track = px_s * px(ids.ind_track);
            let inset = px(ids.ind_inset);
            let text = recase(
                word(ids.ind_case),
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
                // The ring pair and its degrade are `nacelle::plugin_shapes`'s
                // now: a host that carries `ring_fill`/`ring` draws this
                // pill on its own fast path, and a host too old for the
                // pair still bevels it by hand, the same shape this file
                // used to build unconditionally.
                let cut = cut.min(h / 2.0);
                plugin_shapes::ring_fill(api, ctx, pill, CORNER_CHAMFER, cut, fill);
                if !ids.pill_solid && border > 0.0 {
                    plugin_shapes::ring(api, ctx, pill, CORNER_CHAMFER, cut, border, ink(ids.info_edge));
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

/// The session tabs are this widget's only controls, so the hand
/// appears over the strip and nowhere else — over the terminal itself
/// the ordinary pointer is what a terminal wants. The strip metrics are
/// the ones the last frame drew with, exactly as the click uses them.
extern "C" fn pointer_c(
    instance: *mut c_void,
    x: f32,
    y: f32,
    r: RectC,
    _win_w: f32,
    _win_h: f32,
) -> u32 {
    let Some(this) = state(instance) else { return 0 };
    let (pad, tab_h, gap) = this.strip;
    let count = this.tab_count.max(1);
    let over = (0..count)
        .any(|i| contains(&tab_rect(r, pad, tab_h, gap, count, i), x, y));
    u32::from(over)
}

/// Filled, and consumes nothing on purpose — which for a TERMINAL wants
/// saying, because it looks like the one widget that should take every
/// key there is.
///
/// It does not, and cannot: the terminal's state is the HOST's. This
/// file draws a grid of cells the host resolved for it and owns no PTY
/// to write a byte into, so a key taken here would be a key that reached
/// nothing. The bytes go from the host's keyboard straight to the host's
/// terminal, exactly as they did before this entry existed; consuming
/// one would only stop that.
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

/// Filled, and does nothing on purpose. The press this panel cares about
/// is the start of a text SELECTION, and that is a gesture — `drag`'s,
/// the single capture path, of which this entry is deliberately not a
/// second. Nothing else here has a press rung: a tab wears idle, hover
/// and selected, none of which a button going down decides.
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
/// `shell.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `shell.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "shell",
    meta: include_str!("../shell.meta"),
    attach: builtin_attach,
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

#[cfg(test)]
mod token_tests {
    use super::*;

    /// The two type BINDINGS this widget follows, and the roles they had
    /// better name. A binding is a word, the word names a role, and the
    /// role names a family — so the chain is walked here exactly as
    /// `Tokens::resolve` walks it, and a master that renamed a role
    /// fails here instead of drawing a strip of nothing.
    #[test]
    fn both_type_bindings_name_roles_the_master_declares() {
        nacelle::theme::load();
        for binding in ["tab.role", "terminal.indicator.role"] {
            let id = nacelle::theme::id(binding).expect(binding);
            let role = nacelle::theme::enum_word_of(id).expect("the binding names no word");
            assert!(!role.is_empty(), "{binding} binds to nothing");
            for suffix in ["size", "min_px", "tracking", "leading", "case"] {
                let name = role_token(&role, suffix).expect("a bound role names its family");
                assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
            }
        }
    }

    /// The scroll readout is set in the role the TERMINAL binds, and the
    /// master binds a different one from the badge's — which is the
    /// finding: the pill was drawn in `type.badge` while the master said
    /// `terminal.indicator.role`. If the two ever agree again this test
    /// stops proving anything, so it says which is which out loud.
    #[test]
    fn the_scroll_readout_is_not_set_in_the_badge_role() {
        nacelle::theme::load();
        let ind = nacelle::theme::id("terminal.indicator.role").expect("indicator role");
        let badge = nacelle::theme::id("badge.role").expect("badge role");
        let ind = nacelle::theme::enum_word_of(ind).expect("no word");
        let badge = nacelle::theme::enum_word_of(badge).expect("no word");
        assert_ne!(ind, badge, "the master now binds both to {ind}");
        let t = nacelle::theme::resolved();
        let size = |role: &str| {
            t.px(nacelle::theme::id(&role_token(role, "size").unwrap()).expect("size"))
        };
        // And the two roles really are different sizes, so the fix is
        // visible on the first screen rather than only in the names.
        assert_ne!(size(&ind), size(&badge));
    }

    /// The caret is the shape `term.cursor.style` names, cut from the
    /// lengths `[terminal]` keeps beside it.
    ///
    /// All three shapes in one test because the finding is that there
    /// was only ever one: this widget drew a full block whatever the
    /// master said, so `term.cursor.style` and the three lengths
    /// `terminal.cursor.*` had no reader at all (audit 2026-08-17, Z17).
    ///
    /// The three lengths are measured with numbers that are all
    /// DIFFERENT, and different from the cell's, before the master's own
    /// are checked. The master bakes `terminal.cursor.underline_h` and
    /// `terminal.cursor.bar_w` from one `@stroke.thin`, so a shape cut
    /// from the wrong one of the two would measure right anyway — the
    /// test would be reading a coincidence in the theme instead of the
    /// code. (Found by the adversary, 2026-08-17: swapping the two
    /// fields inside `cursor_quad` passed.)
    #[test]
    fn the_caret_is_the_shape_the_master_names_at_the_lengths_beside_it() {
        nacelle::theme::load();
        let t = nacelle::theme::resolved();
        let id = |n: &str| {
            nacelle::theme::id(n).unwrap_or_else(|| panic!("the master declares no {n}"))
        };
        // A cell and three lengths of no round numbers, no two alike, so
        // nothing can match by accident.
        let cell = RectC { x: 11.0, y: 23.0, w: 9.0, h: 21.0 };
        let g = CursorGeom { underline_h: 3.0, underline_gap: 5.0, bar_w: 7.0, lead: 0.0 };

        let (q, covers) = cursor_quad(CursorStyle::Block, cell, g);
        assert!(covers, "a block hides the glyph and has to lay it back over itself");
        assert_eq!((q.x, q.y, q.w, q.h), (cell.x, cell.y, cell.w, cell.h));

        // THE ONE THE FINDING IS ABOUT: a bar of terminal.cursor.bar_w.
        let (q, covers) = cursor_quad(CursorStyle::Bar, cell, g);
        assert!(!covers, "a bar stands beside the glyph, so it must not redraw it");
        assert_eq!(q.w, g.bar_w, "the bar is not terminal.cursor.bar_w wide");
        assert!(q.w < cell.w, "the bar filled its cell, which is a block by another name");
        assert_eq!((q.x, q.y, q.h), (cell.x, cell.y, cell.h));

        let (q, covers) = cursor_quad(CursorStyle::Underline, cell, g);
        assert!(!covers, "an underline sits under the glyph, so it must not redraw it");
        assert_eq!(q.h, g.underline_h, "the caret's rule is not terminal.cursor.underline_h");
        assert!(q.h < cell.h, "the underline filled its cell");
        assert_eq!(
            q.y,
            cell.y + cell.h - g.underline_gap - g.underline_h,
            "the caret's rule ignores terminal.cursor.underline_gap"
        );
        assert_eq!((q.x, q.w), (cell.x, cell.w));

        // And the master's own lengths are real numbers that reach the
        // same arithmetic — the synthetic three above prove which field
        // is read, these prove the fields are filled from the theme.
        let baked = CursorGeom {
            underline_h: t.px(id("terminal.cursor.underline_h")),
            underline_gap: t.px(id("terminal.cursor.underline_gap")),
            bar_w: t.px(id("terminal.cursor.bar_w")),
            lead: 0.0,
        };
        // Below a device pixel the floor in `cursor_quad` would answer
        // instead of the token. The master ships @stroke.thin, which is
        // more.
        assert!(baked.bar_w >= 1.0, "terminal.cursor.bar_w bakes to {} px", baked.bar_w);
        assert!(
            baked.underline_h >= 1.0,
            "terminal.cursor.underline_h is {} px",
            baked.underline_h
        );
        assert_eq!(cursor_quad(CursorStyle::Bar, cell, baked).0.w, baked.bar_w);
        assert_eq!(
            cursor_quad(CursorStyle::Underline, cell, baked).0.h,
            baked.underline_h
        );

        // And the WORD is what chooses between the three.
        let word = nacelle::theme::enum_word_of(id("term.cursor.style"))
            .expect("term.cursor.style names no shape");
        assert_eq!(
            CursorStyle::of(&word),
            CursorStyle::Block,
            "the master now ships `{word}`, so the default screen changed shape"
        );
        assert_eq!(CursorStyle::of("bar"), CursorStyle::Bar);
        assert_eq!(CursorStyle::of("underline"), CursorStyle::Underline);
        // A word this build predates, a misspelling, or no word at all
        // is still a caret — and it is the one that has always been
        // drawn. `beam` is in that company on purpose: one shape has one
        // name in this master, and here it is `bar`.
        assert_eq!(CursorStyle::of("beam"), CursorStyle::Block);
        assert_eq!(CursorStyle::of(""), CursorStyle::Block);
    }

    /// An opened-up row keeps its glyph in the middle, and every rule
    /// that belongs to the glyph travels with it.
    ///
    /// `terminal.line_height` multiplies the CELL, and the cell is the
    /// box the host reports and this file draws in. Before this was
    /// worked out, the only value that did anything to the picture broke
    /// it: the glyph stayed at the cell's ceiling while the underlines
    /// stayed on its floor, so at 2.00 an SGR-4 rule sat some twenty
    /// pixels under the character it belonged to (adversary, 2026-08-17).
    #[test]
    fn the_air_in_a_row_is_shared_and_the_rules_stay_under_the_glyph() {
        // The master's own row at the 1080p reference, near enough: the
        // numbers only have to be a plausible cell.
        const CELL_H: f32 = 20.671202;
        const GAP: f32 = 1.62;
        const RULE: f32 = 1.62;

        // 1.00 — the master's value — is no air at all, and the picture
        // is the one this widget has always drawn.
        assert_eq!(half_leading(CELL_H, 1.00), 0.0);
        assert_eq!(
            underline_y(7.0, CELL_H, half_leading(CELL_H, 1.00), GAP, RULE),
            7.0 + CELL_H - GAP - RULE,
            "the ordinary row moved when nothing about it changed"
        );

        // 2.00: the cell is twice the line box, so a quarter of the cell
        // is air above the glyph and a quarter below.
        let tall = CELL_H * 2.0;
        let lead = half_leading(tall, 2.00);
        assert!(
            (lead - tall / 4.0).abs() < 0.001,
            "half the air in a doubled row came out {lead} px, not {} px",
            tall / 4.0
        );
        // The rule is under the GLYPH: the same distance below the line
        // box's floor as in a row with no air in it at all.
        let y = underline_y(7.0, tall, lead, GAP, RULE);
        assert!(
            (y - (7.0 + lead + CELL_H - GAP - RULE)).abs() < 0.001,
            "the rule landed at {y} px, {} px from where the glyph is",
            y - (7.0 + lead + CELL_H - GAP - RULE)
        );
        assert!(
            y + RULE < 7.0 + tall - lead + 0.001,
            "the rule fell out of the glyph's line box and into the row's air"
        );

        // The caret's rule goes through the same function, so an
        // underlined character with the cursor on it shows ONE line.
        let cell = RectC { x: 3.0, y: 7.0, w: 9.0, h: tall };
        let g = CursorGeom {
            underline_h: RULE,
            underline_gap: GAP,
            bar_w: 4.0,
            lead,
        };
        assert_eq!(
            cursor_quad(CursorStyle::Underline, cell, g).0.y,
            y,
            "the caret's rule and SGR 4's parted company in an opened-up row"
        );

        // Nonsense cannot lift a glyph out of its row: a squeezed line
        // height, a missing token (0.0) and a NaN all mean no air.
        assert_eq!(half_leading(CELL_H, 0.5), 0.0);
        assert_eq!(half_leading(CELL_H, 0.0), 0.0);
        assert_eq!(half_leading(CELL_H, f32::NAN), 0.0);
        // And air is never more than half the row, whatever the token.
        assert!(half_leading(CELL_H, 1.0e9) <= CELL_H / 2.0);
    }
}

/// The grid as it actually goes down, recorded.
///
/// The keyboard widget's frame tests are the pattern: the host's own
/// theme entries answer for real — they ignore the context, so a null
/// one is enough — and only the drawing entries are replaced. `term_view`
/// is replaced too, because that one DOES need a context and there is no
/// terminal behind this test; a stub that reports one cell is the whole
/// of what the grid loop needs to be watched.
///
/// What this reaches that a test on a pure function cannot: where the
/// glyph is put. The rule under it is arithmetic that can be checked in
/// isolation, but "the two of them stay together" is a statement about
/// the loop, and the loop is what got it wrong.
#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// What the stub host reports the face's own line box measuring, and
    /// one cell's width. Any numbers would do; these are the master's
    /// own mono cell at the 1080p reference, so a failure reads like the
    /// screen it came from.
    const LINE_BOX: f32 = 20.671202;
    const CELL_W: f32 = 9.396;

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Cmd {
        Rect { x: f32, y: f32, w: f32, h: f32 },
        Text { ch: u32, x: f32, y: f32 },
    }

    thread_local! {
        static FRAME: RefCell<Vec<Cmd>> = const { RefCell::new(Vec::new()) };
        /// The multiplier the stub host measured its cell with — the
        /// HOST's half of `terminal.line_height`. The theme's half is
        /// set to the same number by the preview in `frame`, which is
        /// exactly the agreement the widget relies on.
        static ROW: Cell<f32> = const { Cell::new(1.0) };
    }

    extern "C" fn rec_rect(_: *mut c_void, r: RectC, _: ColorC) {
        FRAME.with(|f| f.borrow_mut().push(Cmd::Rect { x: r.x, y: r.y, w: r.w, h: r.h }));
    }

    #[allow(clippy::too_many_arguments)]
    extern "C" fn rec_text(
        _: *mut c_void,
        _: u32,
        _: f32,
        x: f32,
        y: f32,
        text: *const u8,
        len: u32,
        _: ColorC,
        _: f32,
        _: u32,
    ) {
        let s = unsafe { std::slice::from_raw_parts(text, len as usize) };
        let ch = String::from_utf8_lossy(s).chars().next().unwrap_or(' ') as u32;
        FRAME.with(|f| f.borrow_mut().push(Cmd::Text { ch, x, y }));
    }

    /// A terminal of one row and one column, holding an underlined `A`
    /// with the cursor on it.
    extern "C" fn stub_view(
        _: *const c_void,
        _: *mut c_void,
        req: *const TermReqC,
        _: u32,
        out: *mut TermViewC,
        _: u32,
    ) -> u32 {
        let req = unsafe { &*req };
        let mut v = TermViewC::empty();
        v.flags = VIEW_LIVE | VIEW_CURSOR;
        v.cell_w = CELL_W;
        v.cell_h = LINE_BOX * ROW.with(|r| r.get());
        v.px = 15.66;
        v.ascent = 15.97;
        v.cols = 1;
        v.rows = 1;
        v.cursor_ch = b'A' as u32;
        let room = if req.cells.is_null() || req.cell_stride == 0 {
            0
        } else {
            req.cells_bytes as usize / req.cell_stride as usize
        };
        if room == 0 {
            // What the host does with a buffer too small: say so, and
            // let the widget grow it and ask again.
            v.flags |= VIEW_TRUNCATED;
        } else {
            v.view_cols = 1;
            v.view_rows = 1;
            let ink = ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
            let cell = CellC {
                ch: b'A' as u32,
                flags: CELL_UNDERLINE,
                width: 1,
                font: nacelle::font::FONT_MONO,
                reserved: 0,
                fg: ink,
                bg: ink,
            };
            unsafe { std::ptr::write(req.cells as *mut CellC, cell) };
        }
        unsafe { std::ptr::write(out, v) };
        1
    }

    fn px(name: &str) -> f32 {
        nacelle::theme::resolved().px(nacelle::theme::id(name).expect(name))
    }

    /// One frame at a given `terminal.line_height`, and the top-left
    /// corner of the one cell in it.
    fn frame(line_height: f32) -> (Vec<Cmd>, f32, f32) {
        nacelle::theme::load();
        ROW.with(|r| r.set(line_height));
        let value = format!("{line_height:.2}");
        let refused = nacelle::theme::set_preview(&[("terminal.line_height", &value)]);
        assert!(refused.is_empty(), "the engine refused {value}: {refused:?}");

        let api = HostApi {
            rect: rec_rect,
            text: rec_text,
            term_view: stub_view,
            ..*nacelle::plugin::host_api()
        };
        FRAME.with(|f| f.borrow_mut().clear());
        let r = RectC { x: 0.0, y: 0.0, w: 400.0, h: 200.0 };
        let mut s = Shell::new();
        s.draw(&api, std::ptr::null_mut(), std::ptr::null(), r);
        let cmds = FRAME.with(|f| f.borrow().clone());
        // The grid's own corner, by the same three tokens `draw` adds up.
        let cell = (
            r.x + px("terminal.pad"),
            r.y + px("tab.pad") + px("tab.h") + px("tab.rule_gap"),
        );
        nacelle::theme::clear_preview();
        (cmds, cell.0, cell.1)
    }

    /// The glyph and the rule under it are one thing, at any line height.
    ///
    /// `terminal.line_height` was the only token in this batch that
    /// changes the picture on its own, and the picture it made was
    /// broken: the host's cell grew, the glyph stayed at the cell's
    /// ceiling and the underline stayed on its floor, so at 2.00 the
    /// rule sat about twenty pixels under the character it belonged to
    /// (adversary, 2026-08-17).
    #[test]
    fn an_opened_up_row_keeps_its_glyph_and_its_rule_together() {
        let rule_h = px("terminal.cell_underline_h");
        let rule_gap = px("terminal.cell_underline_gap");
        assert!(rule_h > 0.0, "the master's SGR-4 rule has no thickness to find");

        // The glyph's own anchor, and the rule's, out of one frame.
        let glyph_and_rule = |cmds: &[Cmd], cell_y: f32, cell_h: f32| -> (f32, f32) {
            let g = cmds
                .iter()
                .find_map(|c| match *c {
                    Cmd::Text { ch, y, .. } if ch == b'A' as u32 => Some(y),
                    _ => None,
                })
                .expect("the frame drew no glyph at all");
            let r = cmds
                .iter()
                .find_map(|c| match *c {
                    // The rule: one cell wide, the token's thickness,
                    // somewhere inside the row.
                    Cmd::Rect { y, w, h, .. }
                        if (w - CELL_W).abs() < 0.001
                            && (h - rule_h).abs() < 0.001
                            && y > cell_y
                            && y < cell_y + cell_h =>
                    {
                        Some(y)
                    }
                    _ => None,
                })
                .expect("the frame drew no underline under an underlined cell");
            (g, r)
        };

        // The master's own row: no air, and the picture this widget has
        // always drawn — glyph at the cell's top, rule on its floor.
        let (cmds, cx, cy) = frame(1.00);
        let (g1, r1) = glyph_and_rule(&cmds, cy, LINE_BOX);
        assert!(
            cmds.contains(&Cmd::Text { ch: b'A' as u32, x: cx, y: cy }),
            "the plain row put its glyph at {g1}, not at the cell's top {cy}"
        );
        assert!(
            (r1 - (cy + LINE_BOX - rule_gap - rule_h)).abs() < 0.001,
            "the plain row's rule moved to {r1} from {}",
            cy + LINE_BOX - rule_gap - rule_h
        );

        // Twice the row. The cell is twice as tall, the glyph drops by a
        // quarter of it, and the rule keeps its distance FROM THE GLYPH.
        let (cmds, _, cy) = frame(2.00);
        let tall = LINE_BOX * 2.0;
        let (g2, r2) = glyph_and_rule(&cmds, cy, tall);
        let air = tall / 4.0;
        assert!(
            (g2 - (cy + air)).abs() < 0.001,
            "the doubled row left its glyph at {g2}; the middle of the row is {}",
            cy + air
        );
        assert!(
            ((r2 - g2) - (r1 - g1)).abs() < 0.001,
            "the rule sits {} px under the glyph in a doubled row and {} px in a plain one",
            r2 - g2,
            r1 - g1
        );
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
mod strip_tests {
    use super::*;

    /// The rung an empty slot wears is the rung a full one wears, and
    /// the WORDS are the whole difference.
    ///
    /// Both halves in one test on purpose: either alone is satisfiable
    /// by something nobody wants. Rungs equal and words equal would be a
    /// strip where nothing tells the two apart; words different and
    /// rungs different is what this replaces — disabled at rest, hover
    /// under the pointer, a control lighting up that 5.21 says never
    /// can. (Owner's recording, 2026-08-12; fixed 2026-08-18.)
    #[test]
    fn an_empty_slot_is_told_apart_by_its_words_and_never_by_its_rung() {
        for active in [false, true] {
            for hover in [false, true] {
                let (empty_rung, empty_text) = tab_face(false, active, hover, 1);
                let (full_rung, full_text) = tab_face(true, active, hover, 1);
                assert_eq!(
                    empty_rung, full_rung,
                    "active={active} hover={hover}: the ladder must not know \
                     whether a session is behind the tab"
                );
                assert_ne!(
                    empty_text, full_text,
                    "active={active} hover={hover}: then nothing at all \
                     tells an empty slot apart"
                );
            }
        }
        // And the ladder is the ordinary one, spelled out so a rung
        // swapped for another still fails here.
        assert_eq!(tab_face(false, false, false, 1).0, ST_IDLE);
        assert_eq!(tab_face(false, false, true, 1).0, ST_HOVER);
        assert_eq!(tab_face(false, true, false, 1).0, ST_SELECTED);
        assert_eq!(tab_face(false, true, true, 1).0, ST_SELECTED_HOVER);
    }

    /// No state this widget can be in reaches the disabled rung.
    ///
    /// The index is taken from the TOOLKIT's ladder rather than written
    /// as 6 here: these numbers cross the plugin boundary as raw u32,
    /// and a test that spelled the number itself would go on passing the
    /// day the enum gained an eighth rung in the middle.
    #[test]
    fn no_tab_this_widget_draws_reaches_the_disabled_rung() {
        let disabled = nacelle::theme::parse::State::Disabled as u32;
        for occupied in [false, true] {
            for active in [false, true] {
                for hover in [false, true] {
                    let (rung, text) = tab_face(occupied, active, hover, 0);
                    assert_ne!(
                        rung, disabled,
                        "occupied={occupied} active={active} hover={hover} \
                         ({text}) rests on the rung that must never light up"
                    );
                }
            }
        }
    }
}
