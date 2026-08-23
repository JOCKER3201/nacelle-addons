//! The on-screen keyboard, as a compiled widget.
//!
//! A script could describe what this looks like but not what it does:
//! sticky SHIFT/CTRL/ALT/FN, control codes built by masking a byte,
//! FN turning digits into function-key sequences, and a key layout that
//! relabels itself while FN is held. That is behaviour, not drawing.
//!
//! Everything below the drawing is the widget as it always was; only
//! the way it puts pixels on screen goes through the host's table now.

use nacelle::runtime::{
    ActionC, ChromeC, ColorC, HostApi, PluginApi, RectC, StateStyleC, ABI_VERSION, ACTION_BYTES,
    ACTION_NONE, CORNER_ROUND,
};
use nacelle::widget::factory::BuiltinWidget;
use std::borrow::Cow;
use std::ffi::c_void;
use std::time::Instant;

/// The font slots, as the host numbers them — the theme's own
/// `FACE_UI = 0` and `FACE_MONO = 1`. The ABI carries these two and
/// clamps anything past them, so a slot is chosen by the WORD a role's
/// `face` names and never by an index into the theme's eight faces.
const FONT_UI: u32 = 0;
const FONT_MONO: u32 = 1;

/// A rectangle in the widget's own arithmetic. The host's `RectC` is
/// what crosses the boundary; this is what the layout is worked out in.
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
}

/// No ink at all, for a host without the token entries (abi_version < 5)
/// — one that cannot be asked what anything looks like.
///
/// Not a grey: a chosen grey is a design decision taken where the theme
/// cannot be reached, and a keyboard drawn in it is an interface nobody
/// designed. Paired with zero widths below and with the zero lengths
/// every accessor then answers, so the whole widget draws NOTHING — the
/// clean bail `ai` takes for the same host.
const NO_INK: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
const RAW_STYLE: StateStyleC = StateStyleC {
    fill: NO_INK,
    edge: NO_INK,
    text: NO_INK,
    glyph: NO_INK,
    edge_width: 0.0,
    glow_radius: 0.0,
    glow_alpha: 0.0,
    elevation: 0.0,
};

/// The state matrix's rungs, in the order `theme_class_state` indexes
/// them: idle, hover, press, selected, selected_hover, dragging, disabled.
const ST_IDLE: u32 = 0;
const ST_HOVER: u32 = 1;
const ST_PRESS: u32 = 2;
const ST_SELECTED: u32 = 3;
const ST_SELECTED_HOVER: u32 = 4;

fn token(api: &HostApi, name: &str) -> u32 {
    (api.theme_token)(name.as_ptr(), name.len() as u32)
}

/// The WORD an enum token resolves to — ABI 6's `theme_enum_word`. Read
/// with the ids and never in a draw loop: it copies a string. An empty
/// answer (a host whose table ends before the entry, a missing token, a
/// token with no word) is what a caller degrades on.
fn enum_word(api: &HostApi, ctx: *mut c_void, id: u32) -> String {
    if !api.has_theme_enum_word() || id == u32::MAX {
        return String::new();
    }
    let mut buf = [0u8; 64];
    let n = (api.theme_enum_word)(ctx, id, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
}

/// The name of one token of the role a `*_role` binding names.
///
/// `None` for a master that binds no role, which leaves every id MISSING
/// and every accessor on zero — type of no size draws nothing. Naming a
/// role here instead would be this file choosing how a legend is set.
fn role_token(role: &str, suffix: &str) -> Option<String> {
    if role.is_empty() {
        return None;
    }
    Some(format!("type.{role}.{suffix}"))
}

/// The font slot a role's `face` names. A face is an OPEN word set, so
/// it is read as a WORD: the boundary carries two slots and clamps
/// anything past them, which would turn `display` into monospace.
fn face_slot(api: &HostApi, ctx: *mut c_void, id: u32) -> u32 {
    if enum_word(api, ctx, id).starts_with("mono") {
        FONT_MONO
    } else {
        FONT_UI
    }
}

/// A role's case transform, applied here because the text entry draws
/// bytes as given. The indices are the schema's declared order — every
/// `*.case` declares `enum: none | upper | lower | smallcaps`, and
/// `theme_enum` indexes that list. Smallcaps needs per-glyph sizes only
/// the host's font system has; through a single text call the nearest
/// honest reading is capitals.
fn recase(word: u32, s: &str) -> Cow<'_, str> {
    match word {
        1 | 3 => Cow::Owned(s.to_uppercase()), // upper | smallcaps
        2 => Cow::Owned(s.to_lowercase()),     // lower
        _ => Cow::Borrowed(s),                 // none, or a word this build predates
    }
}

/// Which corner of a cap its shifted legend sits in — the four words
/// `keyboard.sub_corner` declares, decoded once beside the ids.
///
/// A word this build has never heard of, or no word at all, is NOT
/// substituted with one of these: it means the master and this file
/// disagree about where the mark goes, and the legend is left undrawn
/// rather than parked in a corner nobody asked for.
#[derive(Clone, Copy, PartialEq)]
enum SubCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

fn sub_corner(word: &str) -> Option<SubCorner> {
    Some(match word {
        "top_left" => SubCorner::TopLeft,
        "top_right" => SubCorner::TopRight,
        "bottom_left" => SubCorner::BottomLeft,
        "bottom_right" => SubCorner::BottomRight,
        _ => return None,
    })
}

/// Where the shifted legend's run starts, and which way it grows:
/// `keyboard.sub_inset_x` off the named vertical edge,
/// `keyboard.sub_inset_y` off the named horizontal one, and the run
/// anchored so that the inset is a gap on BOTH edges the corner names.
/// `line_h` is the sub-role's own line box, which is what keeps a
/// bottom-corner legend inside the cap instead of hanging under it.
/// Returns the host's alignment: 0 left, 2 right.
fn sub_place(c: SubCorner, k: &Rect, dx: f32, dy: f32, line_h: f32) -> (f32, f32, u32) {
    let (x, align) = match c {
        SubCorner::TopLeft | SubCorner::BottomLeft => (k.x + dx, 0),
        SubCorner::TopRight | SubCorner::BottomRight => (k.right() - dx, 2),
    };
    let y = match c {
        SubCorner::TopLeft | SubCorner::TopRight => k.y + dy,
        SubCorner::BottomLeft | SubCorner::BottomRight => k.y + k.h - dy - line_h,
    };
    (x, y, align)
}

/// A `same_as_parent` sentinel — every sentinel bakes negative — falls
/// back to the value the master names as its parent; anything the theme
/// really stated is clamped to a length. The toolkit's own reading of
/// the same sentinel (`object::panel::or_parent`), because a plugin that
/// read it differently would put the same word two widths apart.
fn or_parent(v: f32, parent: f32) -> f32 {
    if v < 0.0 {
        parent
    } else {
        v
    }
}

/// The key field inside the panel's content box.
///
/// `keyboard.pad` is padding around the WHOLE field — all four sides —
/// and `keyboard.gap` is the space between two caps. Until this read
/// the horizontal padding was zero and the vertical one borrowed the
/// gap, so the field touched both side edges however the theme was set.
fn field(r: Rect, pad: f32) -> Rect {
    Rect::new(
        r.x + pad,
        r.y + pad,
        (r.w - 2.0 * pad).max(0.0),
        (r.h - 2.0 * pad).max(0.0),
    )
}

/// Every token this widget reads, resolved by NAME once per theme epoch.
/// Ids are stable per master load, not forever, which is why the epoch
/// rides along: when `theme_epoch` moves, the whole set is looked up
/// again.
#[derive(Clone, Copy)]
struct ThemeIds {
    epoch: u32,
    /// The `key` interaction class — the whole cap (fill, ring, legend,
    /// glyph) is one rung of its ladder per frame.
    class_key: u32,
    gap: u32,
    /// Padding around the whole key field, and the cap's own shape:
    /// `keyboard.key_corner` states the radius, `keyboard.key_border`
    /// the ring's width.
    pad: u32,
    key_corner: u32,
    key_border: u32,
    // The type roles the master BINDS to this widget — keyboard.label_role
    // and keyboard.sub_role name a role, and the role's own family is
    // resolved from the word (ABI 6's theme_enum_word). Naming
    // `type.body.*` here instead, as this file did, is the binding
    // spelled twice: the master's and this file's, and only one of them
    // moves when a theme re-roles the legends.
    label_size: u32,
    label_min_px: u32,
    label_max_px: u32,
    label_tracking: u32,
    label_leading: u32,
    label_case: u32,
    label_font: u32,
    sub_size: u32,
    sub_min_px: u32,
    sub_max_px: u32,
    sub_tracking: u32,
    sub_leading: u32,
    sub_case: u32,
    sub_font: u32,
    sub_fg: u32,
    sub_alpha: u32,
    sub_inset_x: u32,
    sub_inset_y: u32,
    /// Which corner of the cap the shifted legend sits in, as the word
    /// `keyboard.sub_corner` names — decoded here because words are
    /// init-time work.
    sub_corner: Option<SubCorner>,
    snap_px: u32,
    center_mode: u32,
    center_bias: u32,
    arrow_size: u32,
    arrow_min_px: u32,
    mod_dot_color: u32,
    mod_dot: u32,
    mod_dot_min_px: u32,
    press_ms: u32,
    motion_scale: u32,
}

impl ThemeIds {
    /// What a pre-token host answers: no class, no tokens. Every accessor
    /// then degrades to the raw kind defaults above.
    const MISSING: ThemeIds = ThemeIds {
        epoch: 0,
        class_key: u32::MAX,
        gap: u32::MAX,
        pad: u32::MAX,
        key_corner: u32::MAX,
        key_border: u32::MAX,
        label_size: u32::MAX,
        label_min_px: u32::MAX,
        label_max_px: u32::MAX,
        label_tracking: u32::MAX,
        label_leading: u32::MAX,
        label_case: u32::MAX,
        label_font: FONT_UI,
        sub_size: u32::MAX,
        sub_min_px: u32::MAX,
        sub_max_px: u32::MAX,
        sub_tracking: u32::MAX,
        sub_leading: u32::MAX,
        sub_case: u32::MAX,
        sub_font: FONT_UI,
        sub_fg: u32::MAX,
        sub_alpha: u32::MAX,
        sub_inset_x: u32::MAX,
        sub_inset_y: u32::MAX,
        sub_corner: None,
        snap_px: u32::MAX,
        center_mode: u32::MAX,
        center_bias: u32::MAX,
        arrow_size: u32::MAX,
        arrow_min_px: u32::MAX,
        mod_dot_color: u32::MAX,
        mod_dot: u32::MAX,
        mod_dot_min_px: u32::MAX,
        press_ms: u32::MAX,
        motion_scale: u32::MAX,
    };

    fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> ThemeIds {
        // The two bindings, followed to the roles they name. An unbound
        // one gives every id below u32::MAX, which is zero size and no
        // ink: a legend the master says nothing about is not drawn.
        let label = enum_word(api, ctx, token(api, "keyboard.label_role"));
        let sub = enum_word(api, ctx, token(api, "keyboard.sub_role"));
        let of = |role: &str, suffix: &str| match role_token(role, suffix) {
            Some(name) => token(api, &name),
            None => u32::MAX,
        };
        ThemeIds {
            epoch,
            class_key: (api.theme_class)(b"key".as_ptr(), 3),
            gap: token(api, "keyboard.gap"),
            pad: token(api, "keyboard.pad"),
            key_corner: token(api, "keyboard.key_corner"),
            key_border: token(api, "keyboard.key_border"),
            label_size: of(&label, "size"),
            label_min_px: of(&label, "min_px"),
            label_max_px: of(&label, "max_px"),
            label_tracking: of(&label, "tracking"),
            label_leading: of(&label, "leading"),
            label_case: of(&label, "case"),
            label_font: face_slot(api, ctx, of(&label, "face")),
            sub_size: of(&sub, "size"),
            sub_min_px: of(&sub, "min_px"),
            sub_max_px: of(&sub, "max_px"),
            sub_tracking: of(&sub, "tracking"),
            sub_leading: of(&sub, "leading"),
            sub_case: of(&sub, "case"),
            sub_font: face_slot(api, ctx, of(&sub, "face")),
            sub_fg: of(&sub, "fg"),
            sub_alpha: of(&sub, "alpha"),
            sub_inset_x: token(api, "keyboard.sub_inset_x"),
            sub_inset_y: token(api, "keyboard.sub_inset_y"),
            sub_corner: sub_corner(&enum_word(api, ctx, token(api, "keyboard.sub_corner"))),
            snap_px: token(api, "type.snap_px"),
            center_mode: token(api, "rhythm.center_mode"),
            center_bias: token(api, "rhythm.cap_center_bias"),
            arrow_size: token(api, "keyboard.arrow_size"),
            arrow_min_px: token(api, "keyboard.arrow_size_min_px"),
            mod_dot_color: token(api, "keyboard.mod_dot_color"),
            mod_dot: token(api, "keyboard.mod_dot"),
            mod_dot_min_px: token(api, "keyboard.mod_dot_min_px"),
            press_ms: token(api, "motion.press.duration_ms"),
            motion_scale: token(api, "motion.scale"),
        }
    }
}

/// The host's interface, kept from the attach call.
static mut HOST: Option<&'static HostApi> = None;

fn host() -> Option<&'static HostApi> {
    unsafe { HOST }
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

#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    /// Plain character; the second element is the SHIFT variant.
    Char(char, char),
    /// Fixed byte sequence.
    Seq(&'static [u8]),
    Shift,
    Ctrl,
    Alt,
    Fn,
}

pub struct KeyDef {
    pub label: &'static str,
    pub shift_label: &'static str,
    pub w: f32,
    pub action: Action,
}

const fn ch(label: &'static str, c: char, shift_label: &'static str, s: char) -> KeyDef {
    KeyDef { label, shift_label, w: 1.0, action: Action::Char(c, s) }
}

const fn letter(label: &'static str, c: char, s: char) -> KeyDef {
    KeyDef { label, shift_label: "", w: 1.0, action: Action::Char(c, s) }
}

const fn seq(label: &'static str, w: f32, bytes: &'static [u8]) -> KeyDef {
    KeyDef { label, shift_label: "", w, action: Action::Seq(bytes) }
}

const fn modk(label: &'static str, w: f32, action: Action) -> KeyDef {
    KeyDef { label, shift_label: "", w, action }
}

pub fn layout() -> [Vec<KeyDef>; 5] {
    [
        vec![
            seq("ESC", 1.3, b"\x1b"),
            ch("`", '`', "~", '~'),
            ch("1", '1', "!", '!'),
            ch("2", '2', "@", '@'),
            ch("3", '3', "#", '#'),
            ch("4", '4', "$", '$'),
            ch("5", '5', "%", '%'),
            ch("6", '6', "^", '^'),
            ch("7", '7', "&", '&'),
            ch("8", '8', "*", '*'),
            ch("9", '9', "(", '('),
            ch("0", '0', ")", ')'),
            ch("-", '-', "_", '_'),
            ch("=", '=', "+", '+'),
            seq("BACK", 1.8, b"\x7f"),
        ],
        vec![
            seq("TAB", 1.6, b"\t"),
            letter("Q", 'q', 'Q'),
            letter("W", 'w', 'W'),
            letter("E", 'e', 'E'),
            letter("R", 'r', 'R'),
            letter("T", 't', 'T'),
            letter("Y", 'y', 'Y'),
            letter("U", 'u', 'U'),
            letter("I", 'i', 'I'),
            letter("O", 'o', 'O'),
            letter("P", 'p', 'P'),
            ch("[", '[', "{", '{'),
            ch("]", ']', "}", '}'),
            ch("\\", '\\', "|", '|'),
        ],
        vec![
            modk("FN", 1.9, Action::Fn),
            letter("A", 'a', 'A'),
            letter("S", 's', 'S'),
            letter("D", 'd', 'D'),
            letter("F", 'f', 'F'),
            letter("G", 'g', 'G'),
            letter("H", 'h', 'H'),
            letter("J", 'j', 'J'),
            letter("K", 'k', 'K'),
            letter("L", 'l', 'L'),
            ch(";", ';', ":", ':'),
            ch("'", '\'', "\"", '"'),
            seq("ENTER", 2.0, b"\r"),
        ],
        vec![
            modk("SHIFT", 2.4, Action::Shift),
            letter("Z", 'z', 'Z'),
            letter("X", 'x', 'X'),
            letter("C", 'c', 'C'),
            letter("V", 'v', 'V'),
            letter("B", 'b', 'B'),
            letter("N", 'n', 'N'),
            letter("M", 'm', 'M'),
            ch(",", ',', "<", '<'),
            ch(".", '.', ">", '>'),
            ch("/", '/', "?", '?'),
            modk("SHIFT", 2.4, Action::Shift),
        ],
        vec![
            modk("CTRL", 1.6, Action::Ctrl),
            modk("ALT", 1.4, Action::Alt),
            seq("SPACE", 8.0, b" "),
            modk("ALT", 1.2, Action::Alt),
            seq("\u{2190}", 1.0, b"\x1b[D"),
            seq("\u{2193}", 1.0, b"\x1b[B"),
            seq("\u{2191}", 1.0, b"\x1b[A"),
            seq("\u{2192}", 1.0, b"\x1b[C"),
        ],
    ]
}

pub struct Keyboard {
    rows: [Vec<KeyDef>; 5],
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub fn_mod: bool,
    /// Key rectangles from the last frame: (rect, row, column).
    hits: Vec<(Rect, usize, usize)>,
    /// The bytes the last click produced, kept alive until the next one.
    /// The host copies them out before returning, so a single buffer is
    /// all this needs — and it belongs to the plugin, which is the only
    /// side that can promise how long it lives.
    last_bytes: Vec<u8>,
    /// Time of the last press (highlight) per key.
    pressed: std::collections::HashMap<(usize, usize), Instant>,
    /// The resolved token ids, remade whenever the theme epoch moves.
    theme: Option<ThemeIds>,
}

impl Keyboard {
    pub fn new() -> Self {
        Keyboard {
            rows: layout(),
            shift: false,
            ctrl: false,
            alt: false,
            fn_mod: false,
            hits: Vec::new(),
            last_bytes: Vec::new(),
            pressed: std::collections::HashMap::new(),
            theme: None,
        }
    }

    /// The token ids for the current theme, re-resolved when the epoch
    /// moves. Only called on an ABI-5 host — the entries this touches do
    /// not exist on an older table.
    fn ids(&mut self, api: &HostApi, ctx: *mut c_void) -> ThemeIds {
        let epoch = (api.theme_epoch)(ctx);
        match self.theme {
            Some(t) if t.epoch == epoch => t,
            _ => {
                let t = ThemeIds::resolve(api, ctx, epoch);
                self.theme = Some(t);
                t
            }
        }
    }

    /// Highlight the key matching a character from the physical keyboard.
    pub fn flash_char(&mut self, c: char) {
        let lc = c.to_ascii_lowercase();
        for (ri, row) in self.rows.iter().enumerate() {
            for (ki, key) in row.iter().enumerate() {
                if let Action::Char(base, shifted) = key.action {
                    if base == lc || shifted == c {
                        self.pressed.insert((ri, ki), Instant::now());
                        return;
                    }
                }
            }
        }
    }

    pub fn flash_label(&mut self, label: &str) {
        for (ri, row) in self.rows.iter().enumerate() {
            for (ki, key) in row.iter().enumerate() {
                if key.label == label {
                    self.pressed.insert((ri, ki), Instant::now());
                    return;
                }
            }
        }
    }

    /// Click handling; returns bytes to send to the PTY.
    pub fn click(&mut self, x: f32, y: f32) -> Option<Vec<u8>> {
        let hit = self
            .hits
            .iter()
            .find(|(r, _, _)| r.contains(x, y))
            .map(|&(_, ri, ki)| (ri, ki))?;
        let (ri, ki) = hit;
        self.pressed.insert((ri, ki), Instant::now());
        let action = self.rows[ri][ki].action;
        // The sound belongs here rather than at the call site: only the
        // keyboard knows WHICH key was hit, and the sticky modifiers
        // return no bytes at all yet still have to be heard.
        nacelle::sound::emit(match action {
            Action::Seq(b"\r") => nacelle::sound::Event::KeyReturn,
            Action::Seq(b"\x7f") => nacelle::sound::Event::KeyErase,
            _ => nacelle::sound::Event::Key,
        });
        match action {
            Action::Shift => {
                self.shift = !self.shift;
                None
            }
            Action::Ctrl => {
                self.ctrl = !self.ctrl;
                None
            }
            Action::Alt => {
                self.alt = !self.alt;
                None
            }
            Action::Fn => {
                self.fn_mod = !self.fn_mod;
                None
            }
            Action::Char(base, shifted) => {
                let mut out = Vec::new();
                // FN + digit = function key (like eDEX).
                if self.fn_mod {
                    if let Some(fseq) = fn_seq(base) {
                        out.extend_from_slice(fseq);
                        self.clear_sticky();
                        return Some(out);
                    }
                }
                let c = if self.shift { shifted } else { base };
                if self.ctrl {
                    let lc = base.to_ascii_lowercase();
                    if lc.is_ascii_alphabetic() {
                        out.push((lc as u8) & 0x1f);
                    } else if "[\\]^_@ ".contains(lc) {
                        out.push((lc as u8) & 0x1f);
                    } else {
                        out.extend_from_slice(c.to_string().as_bytes());
                    }
                } else {
                    if self.alt {
                        out.push(0x1b);
                    }
                    out.extend_from_slice(c.to_string().as_bytes());
                }
                self.clear_sticky();
                Some(out)
            }
            Action::Seq(s) => {
                let mut out = Vec::new();
                if self.alt {
                    out.push(0x1b);
                }
                if self.ctrl && s.len() == 3 && s[0] == 0x1b && s[1] == b'[' {
                    // Arrow keys: xterm's modifier-parameter form (CSI 1;5 <final>).
                    out.extend_from_slice(&s[..2]);
                    out.extend_from_slice(b"1;5");
                    out.push(s[2]);
                } else if self.ctrl && s.len() == 1 {
                    out.push(s[0] & 0x1f);
                } else {
                    out.extend_from_slice(s);
                }
                self.clear_sticky();
                Some(out)
            }
        }
    }

    fn clear_sticky(&mut self) {
        self.shift = false;
        self.ctrl = false;
        self.alt = false;
        self.fn_mod = false;
    }

    fn draw(&mut self, api: &HostApi, ctx: *mut c_void, r: Rect) {
        self.hits.clear();
        // The token entries start at ABI 5. `runtime::attach` already
        // refuses an older host on the dlopen path and the static link is
        // always current, so this branch never falls in practice — but
        // when it does, every read below degrades to the raw kind
        // defaults, never to the retired hardcoded design.
        let abi5 = api.abi_version >= 5;
        let ids = if abi5 { self.ids(api, ctx) } else { ThemeIds::MISSING };
        let px_of = |id: u32| if abi5 { (api.theme_px)(ctx, id) } else { 0.0 };
        let ink = |id: u32| if abi5 { (api.theme_color)(ctx, id) } else { NO_INK };
        let rung = |s: u32| {
            if !abi5 {
                return RAW_STYLE;
            }
            let mut out = RAW_STYLE;
            (api.theme_class_state)(
                ctx,
                ids.class_key,
                s,
                &mut out,
                std::mem::size_of::<StateStyleC>() as u32,
            );
            out
        };
        // The whole cap — fill, ring, legend, glyph — is one rung of the
        // `key` ladder. The five rungs a keyboard can actually reach are
        // fetched once per frame, not once per cap.
        let idle = rung(ST_IDLE);
        let hover = rung(ST_HOVER);
        let press = rung(ST_PRESS);
        let selected = rung(ST_SELECTED);
        let selected_hover = rung(ST_SELECTED_HOVER);

        // A role's resolved px: its size through its own floor and
        // ceiling, snapped to whole pixels when the theme says so.
        let role = |size: u32, floor: u32, cap: u32| {
            let mut v = px_of(size).max(px_of(floor));
            let c = px_of(cap);
            if c > 0.0 && v > c {
                v = c;
            }
            if abi5 && (api.theme_flag)(ctx, ids.snap_px) != 0 {
                v = v.round();
            }
            v
        };
        let label_px = role(ids.label_size, ids.label_min_px, ids.label_max_px);
        let sub_px = role(ids.sub_size, ids.sub_min_px, ids.sub_max_px);
        // Tracking is in em — a multiple of the role's own px.
        let label_spacing = px_of(ids.label_tracking) * label_px;
        let sub_spacing = px_of(ids.sub_tracking) * sub_px;
        let leading = px_of(ids.label_leading);
        let sub_leading = px_of(ids.sub_leading);
        let label_case = if abi5 { (api.theme_enum)(ctx, ids.label_case) } else { 0 };
        let sub_case = if abi5 { (api.theme_enum)(ctx, ids.sub_case) } else { 0 };
        // Optical centring nudges the run by a fraction of its px.
        // `rhythm.center_mode` declares `enum: optical | geometric`, and
        // `theme_enum` indexes that declared list: optical = 0.
        let bias = if abi5 && (api.theme_enum)(ctx, ids.center_mode) == 0 {
            px_of(ids.center_bias) * label_px
        } else {
            0.0
        };

        // The shifted legend draws in its role's own colour, not the
        // cap's state — a `!` does not brighten because Q is hovered.
        let sub_fg = ink(ids.sub_fg);
        let sub_fg = ColorC { a: sub_fg.a * px_of(ids.sub_alpha), ..sub_fg };
        let sub_dx = px_of(ids.sub_inset_x);
        let sub_dy = px_of(ids.sub_inset_y);

        let dot_c = ink(ids.mod_dot_color);
        let dot_px = px_of(ids.mod_dot).max(px_of(ids.mod_dot_min_px));
        let arrow_px = px_of(ids.arrow_size).max(px_of(ids.arrow_min_px));

        // The press decay, scaled the way every duration is; a motion
        // scale of zero means "jump to the end state" and simply never
        // lights the flash.
        let flash_s = px_of(ids.press_ms) * px_of(ids.motion_scale) / 1000.0;

        let gap = px_of(ids.gap);
        // The cap's shape, stated by the two keys the master gives it:
        // `key_corner` is the radius, `key_border` the ring. The radius
        // is CUT round — `[corner]` states that as a rule of the theme
        // file for every radius with no `*_corner_style` sibling beside
        // it, so it is not this file picking a shape.
        let corner = px_of(ids.key_corner);
        let border = px_of(ids.key_border);
        // `keyboard.pad = same_as_parent` names `keyboard.gap` as that
        // parent, so the field's margin and the space between two caps
        // are one decision until a theme separates them.
        let pad = or_parent(px_of(ids.pad), gap).max(0.0);
        let f = field(r, pad);
        let n_rows = self.rows.len();
        let key_h = (f.h - gap * (n_rows as f32 - 1.0)) / n_rows as f32;
        let now = Instant::now();

        // Where the pointer is, for the hover rung. When the host has no
        // answer the NaN it leaves behind matches no rect.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);

        for (ri, row) in self.rows.iter().enumerate() {
            let total_units: f32 = row.iter().map(|k| k.w).sum::<f32>();
            let unit = (f.w - gap * (row.len() as f32 - 1.0)) / total_units;
            let mut x = f.x;
            let y = f.y + (key_h + gap) * ri as f32;
            for (ki, key) in row.iter().enumerate() {
                let kw = unit * key.w;
                let krect = Rect::new(x, y, kw, key_h);

                // Which rung this cap sits on. A struck key reads as
                // press; a latched modifier is a selection — persistence,
                // not pressure — and pointing at it keeps the mark.
                let flash = self
                    .pressed
                    .get(&(ri, ki))
                    .map(|t| now.duration_since(*t).as_secs_f32() < flash_s)
                    .unwrap_or(false);
                let sticky = matches!(
                    (key.action, self.shift, self.ctrl, self.alt, self.fn_mod),
                    (Action::Shift, true, _, _, _)
                        | (Action::Ctrl, _, true, _, _)
                        | (Action::Alt, _, _, true, _)
                        | (Action::Fn, _, _, _, true)
                );
                let hovered = krect.contains(mx, my);
                let style = if flash {
                    &press
                } else if sticky && hovered {
                    &selected_hover
                } else if sticky {
                    &selected
                } else if hovered {
                    &hover
                } else {
                    &idle
                };

                // The cap, on the shape the master gives it. The rung
                // says what colour the cap and its ring are; the WIDTH of
                // that ring is `keyboard.key_border`, the cap's own key,
                // the way the search field's ring is `field.border` while
                // its wash comes off the ladder.
                //
                // The master states the cap's ring twice — that key and
                // `state.<rung>.edge_width`, which the `key` class
                // inherits — and one of the two has to win. The object's
                // own key does, because `[state]` calls itself "the
                // default EVERY class inherits" and a key written for
                // this object is the specific declaration beside it;
                // that is also how `checkbox.border`, `panel.border`,
                // `menu.border` and `field.border` are read across the
                // toolkit. The cost is stated rather than hidden: the
                // ladder's "selection thickens the ring one step" does
                // not reach a cap, so a latched modifier is marked by
                // `selected.fill`, `selected.edge` and its dot alone —
                // and the ladder's own evidence for that step is image
                // 8, a taskbar icon, not a keyboard. Reading it the
                // other way would leave `keyboard.key_border` with no
                // consumer at all: a key a theme can edit to no effect,
                // which is the defect this pass exists to remove.
                //
                // A host too old for the ring pair draws the flat quad
                // it always did — visibly plainer, never a different
                // design.
                let cell = RectC { x: krect.x, y: krect.y, w: krect.w, h: krect.h };
                if api.has_ring() {
                    (api.ring_fill)(ctx, cell, CORNER_ROUND, corner, style.fill);
                    if border > 0.0 && style.edge.a > 0.0 {
                        (api.ring)(ctx, cell, CORNER_ROUND, corner, border, style.edge);
                    }
                } else {
                    (api.rect)(ctx, cell, style.fill);
                    if border > 0.0 && style.edge.a > 0.0 {
                        (api.rect_outline)(ctx, cell, border, style.edge);
                    }
                }

                // Main label in the center. The four cursor caps are the
                // icon registry's arrow_left / arrow_up / arrow_right /
                // arrow_down slots (u2 §2.11, I11), and these triangles
                // are their BUILT-IN FALLBACK: the master ships every
                // icon.arrow_*.layers empty — the reason the comment
                // here used to give ("the UI font may lack the glyphs")
                // is now the registry's `icon.fallback = builtin` rule —
                // and the engine does not bake icon layers across the
                // ABI yet. Size and colour are already the theme's:
                // keyboard.arrow_size and the rung's glyph. A theme that
                // fills the slot replaces the mark when the layer path
                // lands, exactly as icon.link_badge.layers documents.
                let label = if self.fn_mod {
                    if let Action::Char(b, _) = key.action {
                        fn_label(b).unwrap_or(key.label)
                    } else {
                        key.label
                    }
                } else {
                    key.label
                };
                if let Some(dir) = arrow_dir(label) {
                    let cx = krect.cx();
                    let cy = y + key_h / 2.0;
                    let s = arrow_px;
                    let (a, b, c) = match dir {
                        0 => ([cx - s, cy], [cx + s, cy - s], [cx + s, cy + s]), // ←
                        1 => ([cx, cy - s], [cx - s, cy + s], [cx + s, cy + s]), // ↑
                        2 => ([cx + s, cy], [cx - s, cy - s], [cx - s, cy + s]), // →
                        _ => ([cx, cy + s], [cx - s, cy - s], [cx + s, cy - s]), // ↓
                    };
                    let pts = [a[0], a[1], b[0], b[1], c[0], c[1], c[0], c[1]];
                    (api.quad)(ctx, pts.as_ptr(), style.glyph);
                } else {
                    draw_text(
                        api,
                        ctx,
                        ids.label_font,
                        label_px,
                        krect.cx(),
                        y + (key_h - label_px * leading) / 2.0 + bias,
                        &recase(label_case, label),
                        style.text,
                        label_spacing,
                        1,
                    );
                }
                // The shifted legend, in the corner `keyboard.sub_corner`
                // names. A cap whose corner word this build cannot read
                // draws no legend at all rather than one wherever this
                // file would have put it.
                if let (false, Some(c)) = (key.shift_label.is_empty(), ids.sub_corner) {
                    let (sx, sy, align) =
                        sub_place(c, &krect, sub_dx, sub_dy, sub_px * sub_leading);
                    draw_text(
                        api,
                        ctx,
                        ids.sub_font,
                        sub_px,
                        sx,
                        sy,
                        &recase(sub_case, key.shift_label),
                        sub_fg,
                        sub_spacing,
                        align,
                    );
                }
                // The latched-modifier dot: bottom-centre of the cap,
                // standing off the floor by `keyboard.sub_inset_y`.
                // That key names the OTHER mark inside a cap — the
                // shifted legend — because the master declares no
                // `keyboard.mod_dot_inset_y` for this one, and the whole
                // section states only one vertical inset for a mark on a
                // key. Borrowed and reported, which is what this tree
                // does where a token does not exist at all; the number
                // that used to stand here (twice the dot's own size) was
                // an inset no theme could reach.
                if sticky && dot_px > 0.0 {
                    let dot = RectC {
                        x: krect.cx() - dot_px / 2.0,
                        y: y + key_h - sub_dy - dot_px,
                        w: dot_px,
                        h: dot_px,
                    };
                    (api.rect)(ctx, dot, dot_c);
                }

                self.hits.push((krect, ri, ki));
                x += kw + gap;
            }
        }
    }
}

/// 0 = left, 1 = up, 2 = right, 3 = down.
fn arrow_dir(label: &str) -> Option<u8> {
    match label {
        "\u{2190}" => Some(0),
        "\u{2191}" => Some(1),
        "\u{2192}" => Some(2),
        "\u{2193}" => Some(3),
        _ => None,
    }
}

fn fn_seq(digit: char) -> Option<&'static [u8]> {
    Some(match digit {
        '1' => b"\x1bOP",
        '2' => b"\x1bOQ",
        '3' => b"\x1bOR",
        '4' => b"\x1bOS",
        '5' => b"\x1b[15~",
        '6' => b"\x1b[17~",
        '7' => b"\x1b[18~",
        '8' => b"\x1b[19~",
        '9' => b"\x1b[20~",
        '0' => b"\x1b[21~",
        '-' => b"\x1b[23~",
        '=' => b"\x1b[24~",
        _ => return None,
    })
}

fn fn_label(digit: char) -> Option<&'static str> {
    Some(match digit {
        '1' => "F1",
        '2' => "F2",
        '3' => "F3",
        '4' => "F4",
        '5' => "F5",
        '6' => "F6",
        '7' => "F7",
        '8' => "F8",
        '9' => "F9",
        '0' => "F10",
        '-' => "F11",
        '=' => "F12",
        _ => return None,
    })
}


// ----------------------------------------------------------- plugin

extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Keyboard::new())) as *mut c_void
}

extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut Keyboard) });
    }
}

fn state<'a>(instance: *mut c_void) -> Option<&'a mut Keyboard> {
    unsafe { (instance as *mut Keyboard).as_mut() }
}

extern "C" fn draw_c(
    instance: *mut c_void,
    ctx: *mut c_void,
    _host: *const c_void,
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
    let (Some(this), Some(out)) = (state(instance), unsafe { out.as_mut() }) else {
        return;
    };
    match this.click(x, y) {
        Some(bytes) => {
            // The bytes are kept here until the next click, which is
            // what the host reads them from: it copies them out before
            // returning, so one buffer is enough.
            this.last_bytes = bytes;
            out.kind = ACTION_BYTES;
            out.data = this.last_bytes.as_ptr();
            out.data_len = this.last_bytes.len() as u32;
        }
        None => out.kind = ACTION_NONE,
    }
}

extern "C" fn wheel_c(_: *mut c_void, _: f32, _: RectC, _: f32, _: f32, _: *mut ActionC) {}

extern "C" fn grid_c(_: *mut c_void, _: *mut u32, _: *mut u32) {}

extern "C" fn key_feedback_c(
    instance: *mut c_void,
    ch: u32,
    label: *const u8,
    label_len: u32,
) {
    let Some(this) = state(instance) else { return };
    if let Some(c) = char::from_u32(ch) {
        if ch != 0 {
            this.flash_char(c);
        }
    }
    if !label.is_null() && label_len > 0 {
        let bytes = unsafe { std::slice::from_raw_parts(label, label_len as usize) };
        if let Ok(l) = std::str::from_utf8(bytes) {
            this.flash_label(l);
        }
    }
}

/// A grid of keys with proportions of its own: both edges
/// decide how big a key is, which is what the reference box means.
extern "C" fn sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
    nacelle::runtime::SIZING_REFERENCE
}

/// No title band: the keyboard shows no heading today, and a band would
/// take height from the keys.
extern "C" fn chrome_c(
    _: *mut c_void,
    _: *mut c_void,
    _: *const c_void,
    _: *mut ChromeC,
    _: u32,
) -> u32 {
    0
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

/// Filled, and consumes nothing on purpose — emphatically so, because
/// this is the one widget that would be wrong to give a key to.
///
/// An on-screen keyboard's whole job is to SHOW the key somebody else is
/// typing, and that arrives on `key_feedback`, the broadcast every
/// instance hears. This entry is its opposite: it is delivered to the
/// widget that owns the keyboard, and a keyboard that owned the keyboard
/// would be lighting up its own reflection while the field the user is
/// typing into got nothing. Answering 0 keeps every key going where it
/// was going.
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

/// Filled, and does nothing on purpose. A key here lights up for
/// `motion.press.duration_ms` from the moment it is struck — by the
/// pointer or by the physical keyboard, through the broadcast — which is
/// a decay this widget draws from its own clock. A press taken here as
/// well would be a second source of one state, and the two would
/// disagree the first time a key was released somewhere else.
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
/// `keyboard.so` from the addons directory. The name and the metadata
/// are the addon's own — the same string the file would be called and
/// the very bytes of `keyboard.meta` beside it — so a host never
/// describes a widget it merely links: it hands this constant over
/// whole and learns everything from it.
pub const WIDGET: BuiltinWidget = BuiltinWidget {
    name: "keyboard",
    meta: include_str!("../keyboard.meta"),
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

    /// Every token name this widget asks for by a name of its own,
    /// spelled exactly as the code spells it. A name the master does not
    /// declare answers `u32::MAX`, `theme_px` answers zero, and the cap
    /// silently loses its shape — so a typo fails here or nowhere.
    const TOKENS: &[&str] = &[
        "keyboard.gap",
        "keyboard.pad",
        "keyboard.key_corner",
        "keyboard.key_border",
        "keyboard.label_role",
        "keyboard.sub_role",
        "keyboard.sub_corner",
        "keyboard.sub_inset_x",
        "keyboard.sub_inset_y",
        "keyboard.mod_dot",
        "keyboard.mod_dot_min_px",
        "keyboard.mod_dot_color",
        "keyboard.arrow_size",
        "keyboard.arrow_size_min_px",
        "rhythm.center_mode",
        "rhythm.cap_center_bias",
        "type.snap_px",
        "motion.press.duration_ms",
        "motion.scale",
    ];

    #[test]
    fn every_token_this_widget_names_is_one_the_master_declares() {
        nacelle::theme::load();
        let missing: Vec<&str> =
            TOKENS.iter().copied().filter(|n| nacelle::theme::id(n).is_none()).collect();
        assert!(missing.is_empty(), "the master declares no {missing:?}");
    }

    /// The two role bindings are followed to the end, exactly as
    /// `ThemeIds::resolve` follows them: a role the master does not
    /// declare is a legend with no size and no ink, drawn as nothing.
    #[test]
    fn both_legend_roles_are_roles_the_master_declares() {
        nacelle::theme::load();
        for binding in ["keyboard.label_role", "keyboard.sub_role"] {
            let id = nacelle::theme::id(binding).expect(binding);
            let role = nacelle::theme::enum_word_of(id).expect("the binding names no word");
            assert!(!role.is_empty(), "{binding} binds to nothing");
            for suffix in
                ["size", "min_px", "max_px", "tracking", "leading", "case", "face", "fg", "alpha"]
            {
                let name = role_token(&role, suffix).expect("a bound role names its family");
                assert!(nacelle::theme::id(&name).is_some(), "the master declares no {name}");
            }
        }
    }

    /// `keyboard.sub_corner`'s live word is one this file can place. The
    /// decode has no default, so a master naming a fifth word would put
    /// the legend nowhere — and this is where that shows up.
    #[test]
    fn the_sub_legend_corner_word_is_one_this_file_can_place() {
        nacelle::theme::load();
        let id = nacelle::theme::id("keyboard.sub_corner").expect("keyboard.sub_corner");
        let word = nacelle::theme::enum_word_of(id).expect("no word");
        assert!(sub_corner(&word).is_some(), "unplaceable corner word {word:?}");
    }

    /// The cap's radius and ring are lengths the master really answers
    /// with, and the radius is not zero — the value this file used to
    /// draw with when it drew a plain rectangle.
    #[test]
    fn the_cap_carries_a_radius_and_a_ring_the_theme_states() {
        nacelle::theme::load();
        let t = nacelle::theme::resolved();
        let radius = t.px(nacelle::theme::id("keyboard.key_corner").expect("key_corner"));
        let border = t.px(nacelle::theme::id("keyboard.key_border").expect("key_border"));
        assert!(radius > 0.0, "a cap drawn by api.rect had radius 0; the token says {radius}");
        assert!(border > 0.0, "a ring of no width is a cap with no ring");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// `keyboard.pad` is padding on all four sides, and the field is
    /// what the rows are laid out in. Feeding a different pad moves and
    /// resizes the field — which is the whole of the token's job.
    #[test]
    fn the_key_field_is_inset_by_keyboard_pad_on_every_side() {
        let r = Rect::new(10.0, 20.0, 300.0, 200.0);
        let tight = field(r, 0.0);
        assert_eq!((tight.x, tight.y, tight.w, tight.h), (10.0, 20.0, 300.0, 200.0));
        let padded = field(r, 5.0);
        assert_eq!((padded.x, padded.y, padded.w, padded.h), (15.0, 25.0, 290.0, 190.0));
        // A pad wider than the box leaves no field rather than a
        // negative one, which would draw caps outside the panel.
        let over = field(r, 400.0);
        assert!(over.w == 0.0 && over.h == 0.0);
    }

    /// Rows fill the field exactly: five caps and the four gaps between
    /// them, with the padding — and nothing else — on the outside.
    #[test]
    fn five_rows_and_four_gaps_fill_the_padded_field() {
        let r = Rect::new(0.0, 0.0, 400.0, 300.0);
        let (pad, gap, rows) = (7.0, 3.0, 5.0);
        let f = field(r, pad);
        let key_h = (f.h - gap * (rows - 1.0)) / rows;
        let bottom = f.y + (key_h + gap) * (rows - 1.0) + key_h;
        assert!((bottom - (r.y + r.h - pad)).abs() < 0.001);
    }

    /// The four words of `keyboard.sub_corner` put the legend in four
    /// different places, and each one keeps both insets as gaps from the
    /// edges its word names.
    #[test]
    fn each_corner_word_puts_the_shifted_legend_somewhere_else() {
        let k = Rect::new(100.0, 50.0, 40.0, 30.0);
        let (dx, dy, line) = (4.0, 3.0, 10.0);
        let tl = sub_place(SubCorner::TopLeft, &k, dx, dy, line);
        let tr = sub_place(SubCorner::TopRight, &k, dx, dy, line);
        let bl = sub_place(SubCorner::BottomLeft, &k, dx, dy, line);
        let br = sub_place(SubCorner::BottomRight, &k, dx, dy, line);
        assert_eq!(tl, (104.0, 53.0, 0));
        assert_eq!(tr, (136.0, 53.0, 2));
        assert_eq!(bl, (104.0, 67.0, 0));
        assert_eq!(br, (136.0, 67.0, 2));
        // A bottom corner keeps the whole line box inside the cap.
        assert!(bl.1 + line <= k.y + k.h);
        // And the insets are really read: a larger one moves every
        // corner inward.
        let far = sub_place(SubCorner::TopLeft, &k, 12.0, 9.0, line);
        assert!(far.0 > tl.0 && far.1 > tl.1);
    }
}

/// What the widget actually put on the screen, recorded through a host
/// table whose DRAWING entries are these and whose theme entries are the
/// engine's own.
///
/// This is the only way a plugin's picture can be examined from a test:
/// everything it draws leaves through the function table, so a table
/// that writes the calls down instead of rasterising them IS the frame.
/// The theme half is not stubbed — `nacelle::plugin::host_api()` answers
/// from the loaded master — so what these tests compare a shape against
/// is the live value of the token that shaped it.
#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Cmd {
        RingFill { r: [f32; 4], style: u32, radius: f32 },
        Ring { r: [f32; 4], style: u32, radius: f32, w: f32 },
        Rect { r: [f32; 4] },
    }

    thread_local! {
        static FRAME: RefCell<Vec<Cmd>> = const { RefCell::new(Vec::new()) };
    }

    fn push(c: Cmd) {
        FRAME.with(|f| f.borrow_mut().push(c));
    }

    extern "C" fn rec_ring_fill(_: *mut c_void, r: RectC, style: u32, radius: f32, _: ColorC) {
        push(Cmd::RingFill { r: [r.x, r.y, r.w, r.h], style, radius });
    }

    extern "C" fn rec_ring(
        _: *mut c_void,
        r: RectC,
        style: u32,
        radius: f32,
        w: f32,
        _: ColorC,
    ) {
        push(Cmd::Ring { r: [r.x, r.y, r.w, r.h], style, radius, w });
    }

    extern "C" fn rec_rect(_: *mut c_void, r: RectC, _: ColorC) {
        push(Cmd::Rect { r: [r.x, r.y, r.w, r.h] });
    }

    /// One frame of the keyboard, drawn into the recorder.
    fn frame(r: Rect) -> Vec<Cmd> {
        frame_of(Keyboard::new(), r)
    }

    /// The same frame from a keyboard already in some state — a latched
    /// modifier is a state, and its mark only exists in one.
    fn frame_of(mut k: Keyboard, r: Rect) -> Vec<Cmd> {
        nacelle::theme::load();
        let api = HostApi {
            ring_fill: rec_ring_fill,
            ring: rec_ring,
            rect: rec_rect,
            ..*nacelle::plugin::host_api()
        };
        FRAME.with(|f| f.borrow_mut().clear());
        // A null context is what input-time calls already pass: every
        // theme entry ignores it, and the drawing entries here are ours.
        k.draw(&api, std::ptr::null_mut(), r);
        FRAME.with(|f| f.borrow().clone())
    }

    fn px(name: &str) -> f32 {
        nacelle::theme::resolved().px(nacelle::theme::id(name).expect(name))
    }

    /// THE proof for the heavy finding: every cap is drawn as a rounded
    /// ring whose radius is `keyboard.key_corner` and whose stroke is
    /// `keyboard.key_border`, both as the loaded master answers them
    /// right now. Before this change the same frame was `api.rect` plus
    /// `api.rect_outline` — a radius of zero no theme could move.
    #[test]
    fn every_cap_wears_the_radius_and_the_ring_the_theme_states() {
        let cmds = frame(Rect::new(0.0, 0.0, 1200.0, 400.0));
        let (radius, border) = (px("keyboard.key_corner"), px("keyboard.key_border"));
        let fills: Vec<&Cmd> = cmds
            .iter()
            .filter(|c| matches!(c, Cmd::RingFill { .. }))
            .collect();
        // 61 caps in the five rows this widget declares.
        assert_eq!(fills.len(), layout().iter().map(|r| r.len()).sum::<usize>());
        for c in cmds.iter() {
            match c {
                Cmd::RingFill { style, radius: got, .. } => {
                    assert_eq!(*style, CORNER_ROUND);
                    assert_eq!(*got, radius, "the cap's radius is not keyboard.key_corner");
                }
                Cmd::Ring { style, radius: got, w, .. } => {
                    assert_eq!(*style, CORNER_ROUND);
                    assert_eq!(*got, radius);
                    assert_eq!(*w, border, "the cap's ring is not keyboard.key_border");
                }
                // The only plain rectangles left are the latched
                // modifiers' dots, and no modifier is latched here.
                Cmd::Rect { .. } => panic!("a cap drawn as a plain rectangle"),
            }
        }
        assert!(radius > 0.0 && border > 0.0);
    }

    /// The field really is inset by `keyboard.pad`: the leftmost cap
    /// starts a pad in from the box's edge and the rightmost ends a pad
    /// short of it, where both used to sit exactly on it.
    #[test]
    fn the_caps_stop_a_keyboard_pad_short_of_every_edge() {
        let r = Rect::new(30.0, 10.0, 1200.0, 400.0);
        let cmds = frame(r);
        // The master writes this pad as `same_as_parent`, which bakes to
        // a negative sentinel and reads as `keyboard.gap`.
        let pad = or_parent(px("keyboard.pad"), px("keyboard.gap"));
        assert!(pad > 0.0, "neither the pad nor its parent is a length");
        let boxes: Vec<[f32; 4]> = cmds
            .iter()
            .filter_map(|c| match c {
                Cmd::RingFill { r, .. } => Some(*r),
                _ => None,
            })
            .collect();
        let left = boxes.iter().map(|b| b[0]).fold(f32::INFINITY, f32::min);
        let right = boxes.iter().map(|b| b[0] + b[2]).fold(f32::NEG_INFINITY, f32::max);
        let top = boxes.iter().map(|b| b[1]).fold(f32::INFINITY, f32::min);
        let bottom = boxes.iter().map(|b| b[1] + b[3]).fold(f32::NEG_INFINITY, f32::max);
        assert!((left - (r.x + pad)).abs() < 0.01, "left edge at {left}");
        assert!((right - (r.x + r.w - pad)).abs() < 0.01, "right edge at {right}");
        assert!((top - (r.y + pad)).abs() < 0.01, "top edge at {top}");
        assert!((bottom - (r.y + r.h - pad)).abs() < 0.01, "bottom edge at {bottom}");
    }

    /// The latched modifier's dot stands off the cap's floor by a length
    /// the theme states — `keyboard.sub_inset_y`, borrowed from the only
    /// other mark the master places inside a cap, because it declares no
    /// `keyboard.mod_dot_inset_y`. What stood here before was twice the
    /// dot's own size: a distance no theme could reach, and this is what
    /// fails if it comes back.
    #[test]
    fn the_latched_dot_stands_off_the_floor_by_a_length_the_theme_states() {
        let mut k = Keyboard::new();
        k.shift = true;
        let cmds = frame_of(k, Rect::new(0.0, 0.0, 1200.0, 400.0));
        let dots: Vec<[f32; 4]> = cmds
            .iter()
            .filter_map(|c| match c {
                // The caps go through the ring pair, so the only plain
                // rectangles in a latched frame are the dots.
                Cmd::Rect { r } => Some(*r),
                _ => None,
            })
            .collect();
        assert!(!dots.is_empty(), "a latched modifier drew no dot");
        let caps: Vec<[f32; 4]> = cmds
            .iter()
            .filter_map(|c| match c {
                Cmd::RingFill { r, .. } => Some(*r),
                _ => None,
            })
            .collect();
        let inset = px("keyboard.sub_inset_y");
        let size = px("keyboard.mod_dot").max(px("keyboard.mod_dot_min_px"));
        assert!(inset > 0.0 && size > 0.0);
        for d in &dots {
            assert!((d[3] - size).abs() < 0.01, "a dot {} px tall", d[3]);
            let floor = d[1] + d[3] + inset;
            assert!(
                caps.iter().any(|c| ((c[1] + c[3]) - floor).abs() < 0.01),
                "no cap whose floor is a keyboard.sub_inset_y under this dot"
            );
        }
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
