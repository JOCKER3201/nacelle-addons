//! The path box: the toolkit's field, drawn through the plugin ABI.
//!
//! # Why this file exists at all
//!
//! It is the search plugin's `field.rs` again — the acknowledged copy.
//! The model half of `object::text_input` is used verbatim; the VIEW
//! half cannot be, because it draws through `Ctx`, which carries the
//! host's draw list, its font system and its focus chain, and none of
//! those three cross a dynamic library boundary. A plugin has a
//! function table and an opaque handle.
//!
//! So this is that view once more, over [`Surface`] instead of `Ctx`,
//! from the SAME tokens — `[field]` for the geometry, `component.field.*`
//! for the colours, class `field` for the state wash,
//! `motion.caret_blink.*` for the blink. Not one number here is this
//! file's own: a token the master does not declare answers zero and the
//! part it decides simply is not drawn, which is the degradation every
//! widget in this tree shares.
//!
//! It is duplication, and it is worth naming as such: the day the object
//! layer grows a `Surface`-based field — or the ABI grows enough of a
//! focus chain for a plugin to hold a `Ctx` — this file and the search
//! plugin's original are deleted TOGETHER and both calls go to the
//! toolkit's own. Until then, a copy per plugin is the shape the search
//! plugin's header already settled, and the copies must not drift: the
//! wrap-shape tests live with the original, once, rather than once per
//! copy.
//!
//! What is deliberately NOT here: the mask. A path box is never a
//! password box, and a masked field that no test covers is a promise
//! about a secret.

use nacelle::object::text_input::InputModel;
use nacelle::theme::parse::State;
use nacelle::ui::Align;
use nacelle::view::paint;
use nacelle::view::surface::Surface;
use nacelle::Rect;

/// The field's own between-frame state — the two things an immediate
/// mode view cannot recompute from the model, because the model does not
/// keep them where a plugin can reach them.
#[derive(Default)]
pub struct FieldView {
    /// Horizontal offset in px, so a value longer than the box follows
    /// the caret instead of running off the end.
    scroll_px: f32,
    /// What the last frame drew: value, caret, composition. A change is
    /// what restarts the blink, so a caret being typed at is always lit.
    seen: (String, usize, String),
    /// When the blink phase last restarted, on the host's clock.
    blink_t: f64,
    /// Where every caret position of the VALUE landed on screen, as
    /// (window x, byte offset), recorded while drawing.
    ///
    /// A click arrives with no drawing context — the ABI's `click` has
    /// no `ctx` at all — so a plugin cannot measure text when it is
    /// asked where a click landed. It can only remember where it PUT
    /// the text, which is what this is.
    stops: Vec<(f32, usize)>,
}

impl FieldView {
    pub fn new() -> FieldView {
        FieldView::default()
    }

    /// The VALUE byte offset a click at window-x `x` means: the caret
    /// position the last frame drew nearest to it.
    ///
    /// The answer is fed to `InputMsg::Point`, which floors any offset
    /// onto a grapheme boundary — so a position inside a cluster lands
    /// where a segmenter would have put it, and this crate needs none.
    /// Before the first frame there are no positions and the answer is
    /// the start of the value, which is where a caret with no drawing
    /// behind it honestly is.
    pub fn hit(&self, x: f32) -> usize {
        let mut best = 0;
        let mut near = f32::INFINITY;
        for (sx, at) in &self.stops {
            let d = (x - sx).abs();
            if d < near {
                near = d;
                best = *at;
            }
        }
        best
    }
}

/// Whether the caret is lit this instant.
///
/// `motion.caret_blink.*` consumed with a per-field phase, and frozen
/// FULLY VISIBLE when the effect is off or motion is reduced — the
/// project's freeze-at-visible rule, the same one `object::text_input`
/// applies. A caret frozen invisible would be a field that looks dead.
fn lit(sf: &mut impl Surface, since: f64, now: f64) -> bool {
    let scale = sf.px("motion.scale");
    if scale <= 0.0 || !sf.flag("motion.caret_blink.enabled") {
        return true;
    }
    let period = (sf.px("motion.caret_blink.period_ms") * scale) as f64;
    if period <= 0.0 {
        return true;
    }
    let phase = ((now - since) * 1000.0 % period) / period;
    (phase as f32) < sf.px("motion.caret_blink.duty")
}

/// Draws the field.
///
/// `focused` is the caller's answer to "does this box own the keyboard":
/// there is no focus chain across the ABI, so the panel says, exactly as
/// `InputStyle::focused_fallback` lets a modal say on the host side.
pub fn draw(
    sf: &mut impl Surface,
    r: Rect,
    m: &InputModel,
    v: &mut FieldView,
    placeholder: &str,
    focused: bool,
) {
    let now = sf.now();
    let (mx, my) = sf.mouse();
    let hovered = r.contains(mx, my);

    // ---- the box ----------------------------------------------------
    // A LENGTH IS NOT A SHAPE: `field.corner` carries how FAR the corner
    // is cut and `field.corner_style` carries HOW, and both are read
    // through the toolkit's own resolvers so this box and the object
    // layer's field cut their corners by one rule (§5.0's `pill` word
    // included — the story is with the search plugin's original).
    let corner = paint::corner_radius(sf, "field.corner", r, 1.0);
    let cut = paint::corner_style(sf, "field.corner_style");
    let fill = sf.bed("component.field.fill");
    sf.ring_fill(r, cut, corner, fill);
    // The ladder's wash over the bed — idle is a wash too, the button
    // idiom. The field has no press rung of its own; focus is carried by
    // the ring's width below, which is what `field.border_focused` is.
    let state = if hovered { State::Hover } else { State::Idle };
    let wash = sf.class_state("field", state).fill;
    sf.ring_fill(r, cut, corner, wash);
    let bw = sf
        .px(if focused { "field.border_focused" } else { "field.border" })
        .max(0.0);
    if bw > 0.0 {
        let c = sf.color("component.field.border");
        sf.ring(r, cut, corner, bw, c);
    }

    // ---- type metrics -----------------------------------------------
    let role = paint::bound_role(sf, "field.role", 1.0);
    let line_h = role.px * role.leading;
    let ty = paint::center_line_y(sf, r.y, r.h, role.px, role.leading);
    let pad = sf.px("field.pad_x").max(0.0);
    let area = Rect::new(r.x + pad, r.y, (r.w - 2.0 * pad).max(1.0), r.h);

    // ---- what there is to draw ---------------------------------------
    // The composition exists only while the box owns the keyboard: an
    // IME composes into the control that has it.
    let pre = if focused { m.preedit().cloned() } else { None };
    let before = &m.value()[..m.cursor()];
    let after = &m.value()[m.cursor()..];
    let (pre_text, pre_caret) = match &pre {
        Some((p, range)) => {
            // The caret INSIDE the composition, where the platform names
            // one. Its offset is clamped onto a char boundary rather
            // than trusted: an IME's arithmetic is not to be handed a
            // slice index.
            let mut c = range.map(|(a, _)| a.min(p.len())).unwrap_or(p.len());
            while c > 0 && !p.is_char_boundary(c) {
                c -= 1;
            }
            (p.as_str(), c)
        }
        None => ("", 0),
    };
    let mut disp = String::with_capacity(before.len() + pre_text.len() + after.len());
    disp.push_str(before);
    disp.push_str(pre_text);
    disp.push_str(after);
    let caret_at = before.len() + pre_caret;
    // The selection is not drawn under a live composition (the commit
    // replaces it) nor while the box is unfocused. Without a composition
    // the display string IS the value, so its byte offsets are the
    // model's own.
    let sel = if focused && pre.is_none() { m.selection() } else { None };

    // The blink restarts on every change the user can see, so a caret
    // being typed at never blinks out from under the letters.
    let mark = (m.value().to_string(), m.cursor(), pre_text.to_string());
    if v.seen != mark {
        v.seen = mark;
        v.blink_t = now;
    }

    // ---- measure and follow the caret --------------------------------
    // Measured per frame rather than per edit: the toolkit's own view
    // caches this against an edit counter that is private to the model,
    // and a field is one line — three measurements a frame against the
    // hundreds a terminal cell grid already makes.
    let caret_x = sf.measure(role.face, role.px, &disp[..caret_at], role.track);
    let text_w = sf.measure(role.face, role.px, &disp, role.track);
    let margin = sf.px("field.scroll_margin").max(0.0).min(area.w / 2.0);
    if focused {
        if caret_x - v.scroll_px > area.w - margin {
            v.scroll_px = caret_x - (area.w - margin);
        }
        if caret_x - v.scroll_px < margin {
            v.scroll_px = caret_x - margin;
        }
    }
    v.scroll_px = v.scroll_px.clamp(0.0, (text_w - area.w).max(0.0));
    let x0 = area.x - v.scroll_px;

    // ---- text --------------------------------------------------------
    let clipped = sf.clip(Rect::new(area.x, r.y, area.w, r.h));
    if disp.is_empty() {
        v.scroll_px = 0.0;
        if !placeholder.is_empty() {
            let c = sf.color("component.field.placeholder");
            sf.text(role.face, role.px, area.x, ty, placeholder, c, role.track, Align::Left);
        }
    } else {
        let ink = sf.color("component.field.text");
        // The selection wash first, under its own ink.
        if let Some((a, b)) = sel {
            let xa = sf.measure(role.face, role.px, &disp[..a], role.track);
            let xb = sf.measure(role.face, role.px, &disp[..b], role.track);
            let c = sf.color("component.field.selection");
            sf.rect(Rect::new(x0 + xa, ty, xb - xa, line_h), c);
        }
        // The runs: plain / selected / plain, or plain around a live
        // composition. Never both — a composition replaces the selection
        // when it commits, so the two are never live together.
        let mut runs: Vec<(usize, usize, bool)> = Vec::new();
        if let Some((a, b)) = sel {
            runs.push((0, a, false));
            runs.push((a, b, true));
            runs.push((b, disp.len(), false));
        } else if !pre_text.is_empty() {
            let p0 = before.len();
            runs.push((0, p0, false));
            runs.push((p0, p0 + pre_text.len(), true));
            runs.push((p0 + pre_text.len(), disp.len(), false));
        } else {
            runs.push((0, disp.len(), false));
        }
        let marked = if sel.is_some() {
            sf.color("component.field.selection_text")
        } else {
            sf.color("component.field.preedit")
        };
        let composing = sel.is_none() && !pre_text.is_empty();
        let ul = sf.px("field.preedit_underline").max(0.0);
        for (a, b, mark) in runs {
            if a >= b {
                continue;
            }
            let rx = x0 + sf.measure(role.face, role.px, &disp[..a], role.track);
            let c = if mark { marked } else { ink };
            sf.text(role.face, role.px, rx, ty, &disp[a..b], c, role.track, Align::Left);
            if mark && composing && ul > 0.0 {
                // The composition's underline, in the composition's own
                // ink: what marks text as not yet committed.
                let w = sf.measure(role.face, role.px, &disp[a..b], role.track);
                sf.rect(Rect::new(rx, ty + line_h - ul, w, ul), c);
            }
        }
    }

    // ---- caret -------------------------------------------------------
    if focused {
        let ch = sf.px("field.caret_h").max(0.0).min(r.h);
        let cw = sf.px("field.caret_w").max(0.0);
        if ch > 0.0 && cw > 0.0 && lit(sf, v.blink_t, now) {
            // A block or an underline caret is as wide as the grapheme
            // it sits on; past the end of the value it falls back to the
            // space advance, which is a font metric and not a design.
            let word = sf.word("field.caret_style");
            let cx = x0 + caret_x;
            let rect = match word.as_str() {
                "block" | "underline" => {
                    let g = disp[caret_at.min(disp.len())..].chars().next();
                    let s = g.map(String::from).unwrap_or_else(|| " ".to_string());
                    let gw = sf.measure(role.face, role.px, &s, role.track);
                    if word == "block" {
                        Rect::new(cx, r.y + (r.h - ch) / 2.0, gw, ch)
                    } else {
                        Rect::new(cx, r.y + (r.h + ch) / 2.0 - cw, gw, cw)
                    }
                }
                // "bar", and anything the vocabulary does not name.
                _ => Rect::new(cx, r.y + (r.h - ch) / 2.0, cw, ch),
            };
            let c = sf.color("component.field.caret");
            sf.rect(rect, c);
        }
    }
    if clipped {
        sf.unclip();
    }

    // ---- where the caret can be put ----------------------------------
    // Measured from the VALUE and not from the display string: a click
    // during a composition is nobody's — the IME owns the pointer as
    // well as the keyboard until it commits.
    v.stops.clear();
    let mut at = 0usize;
    loop {
        let w = sf.measure(role.face, role.px, &m.value()[..at], role.track);
        v.stops.push((x0 + w, at));
        match m.value()[at..].chars().next() {
            Some(c) => at += c.len_utf8(),
            None => break,
        }
    }
}
