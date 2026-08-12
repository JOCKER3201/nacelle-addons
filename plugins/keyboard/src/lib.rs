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
    ACTION_NONE,
};
use nacelle::widget::factory::BuiltinWidget;
use std::ffi::c_void;
use std::time::Instant;

/// The interface font, as the host numbers them.
const FONT_UI: u32 = 0;

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

/// The engine's raw kind defaults, answered when this copy is attached to
/// a host without the token entries (abi_version < 5). They mirror
/// `StateStyle::RAW` and `ResolvedTheme::RAW_INK`: grey ink, no fill, one
/// hairline — visibly unstyled, never the retired hardcoded design.
const RAW_INK: ColorC = ColorC { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };
const RAW_STYLE: StateStyleC = StateStyleC {
    fill: ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
    edge: RAW_INK,
    text: RAW_INK,
    glyph: RAW_INK,
    edge_width: 1.0,
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
    // The type roles the master binds to this widget (label_role = body,
    // sub_role = caption). The role indirection itself is an enum whose
    // word list never crosses the C ABI, so the roles the master names
    // are read directly.
    label_size: u32,
    label_min_px: u32,
    label_max_px: u32,
    label_tracking: u32,
    label_leading: u32,
    sub_size: u32,
    sub_min_px: u32,
    sub_max_px: u32,
    sub_tracking: u32,
    sub_fg: u32,
    sub_alpha: u32,
    sub_inset_x: u32,
    sub_inset_y: u32,
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
        label_size: u32::MAX,
        label_min_px: u32::MAX,
        label_max_px: u32::MAX,
        label_tracking: u32::MAX,
        label_leading: u32::MAX,
        sub_size: u32::MAX,
        sub_min_px: u32::MAX,
        sub_max_px: u32::MAX,
        sub_tracking: u32::MAX,
        sub_fg: u32::MAX,
        sub_alpha: u32::MAX,
        sub_inset_x: u32::MAX,
        sub_inset_y: u32::MAX,
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

    fn resolve(api: &HostApi, epoch: u32) -> ThemeIds {
        ThemeIds {
            epoch,
            class_key: (api.theme_class)(b"key".as_ptr(), 3),
            gap: token(api, "keyboard.gap"),
            label_size: token(api, "type.body.size"),
            label_min_px: token(api, "type.body.min_px"),
            label_max_px: token(api, "type.body.max_px"),
            label_tracking: token(api, "type.body.tracking"),
            label_leading: token(api, "type.body.leading"),
            sub_size: token(api, "type.caption.size"),
            sub_min_px: token(api, "type.caption.min_px"),
            sub_max_px: token(api, "type.caption.max_px"),
            sub_tracking: token(api, "type.caption.tracking"),
            sub_fg: token(api, "type.caption.fg"),
            sub_alpha: token(api, "type.caption.alpha"),
            sub_inset_x: token(api, "keyboard.sub_inset_x"),
            sub_inset_y: token(api, "keyboard.sub_inset_y"),
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

fn draw_text(
    api: &HostApi,
    ctx: *mut c_void,
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
        FONT_UI,
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
                let t = ThemeIds::resolve(api, epoch);
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
                out.extend_from_slice(s);
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
        let ink = |id: u32| if abi5 { (api.theme_color)(ctx, id) } else { RAW_INK };
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
        let n_rows = self.rows.len();
        let key_h = (r.h - gap * (n_rows as f32 + 1.0)) / n_rows as f32;
        let now = Instant::now();

        // Where the pointer is, for the hover rung. When the host has no
        // answer the NaN it leaves behind matches no rect.
        let (mut mx, mut my) = (f32::NAN, f32::NAN);
        (api.mouse)(ctx, &mut mx, &mut my);

        for (ri, row) in self.rows.iter().enumerate() {
            let total_units: f32 = row.iter().map(|k| k.w).sum::<f32>();
            let unit = (r.w - gap * (row.len() as f32 - 1.0)) / total_units;
            let mut x = r.x;
            let y = r.y + gap + (key_h + gap) * ri as f32;
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

                let cell = RectC { x: krect.x, y: krect.y, w: krect.w, h: krect.h };
                (api.rect)(ctx, cell, style.fill);
                (api.rect_outline)(ctx, cell, style.edge_width, style.edge);

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
                        label_px,
                        krect.cx(),
                        y + (key_h - label_px * leading) / 2.0 + bias,
                        label,
                        style.text,
                        label_spacing,
                        1,
                    );
                }
                // SHIFT variant in the top-right corner — the only corner
                // the master names; `keyboard.sub_corner`'s other words
                // cannot be told apart across the ABI.
                if !key.shift_label.is_empty() {
                    draw_text(
                        api,
                        ctx,
                        sub_px,
                        krect.right() - sub_dx,
                        y + sub_dy,
                        key.shift_label,
                        sub_fg,
                        sub_spacing,
                        2,
                    );
                }
                // The latched-modifier dot, finally drawn: bottom-centre
                // of the cap, spaced off the edge by its own size — the
                // marker the theme declared and no widget read until now.
                if sticky && dot_px > 0.0 {
                    let dot = RectC {
                        x: krect.cx() - dot_px / 2.0,
                        y: y + key_h - 2.0 * dot_px,
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
