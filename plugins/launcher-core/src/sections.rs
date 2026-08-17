//! The alphabetical index: what a letter GROUP is, where the headings
//! fall in a scrolling column of tiles, and the one heading's look.
//!
//! The grid draws this only when the whole menu is on show. Pointed at
//! one category ([`crate::selection`]) it draws a flat page, because a
//! group of eleven applications broken into eight lettered sections is
//! an index of nothing.
//!
//! Two things live here and they are the same thing seen twice: [`of`]
//! says where the breaks in a list of names ARE, and [`plan`] turns
//! those breaks into a column of bands — a heading, then that group's
//! rows of tiles, then the next heading. Nothing here draws except
//! [`head`], and nothing here decides a colour or a length: every one
//! arrives from the theme through [`HeadLook`].

use crate::tile::{self, Rect, TileLook};
use nacelle::runtime::{ColorC, HostApi, RectC};
use std::ffi::c_void;

/// Where a name that does not begin with a letter is filed.
///
/// One group and not ten, because `0 A.D.`, `2048` and `7-Zip` under
/// three separate rules is an index that costs three headings to say
/// nothing; and because a launcher's reader looks under a LETTER, so
/// everything that is not one is the same kind of answer to them.
///
/// `#` rather than a word, for the same reason the tile's icon stand-in
/// is a letter: it is a mark, not a translated string, and this tree
/// has nowhere to translate a string to yet.
pub const NON_LETTER: char = '#';

/// The letter a display name is filed under.
///
/// Uppercased with the standard mapping, so `ł` and `Ł` are one group,
/// and per-character rather than per-string: a name whose first letter
/// uppercases to several (`ß` to `SS`) is filed under the first of
/// them, which is where a reader would look for it.
///
/// The diacritics keep their OWN letters — `Ł`, `Ż`, `Ó` are three
/// headings and not one, and never a bag of "the rest". Folding them
/// into `L`, `Z` and `O` would be a collation decision, and a wrong one
/// in the locale that has those letters: in Polish `Ł` follows `L` as
/// its own letter. Whether it SORTS there is [`crate::desktop`]'s
/// question, not this file's — see [`of`].
pub fn key(name: &str) -> char {
    match name.chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().next().unwrap_or(c),
        _ => NON_LETTER,
    }
}

/// One run of the list that shares a letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// The letter over it, as [`key`] answered.
    pub key: char,
    /// Where the run starts, as a place in the list [`of`] was given.
    pub start: usize,
    /// How many names are in it. Never zero.
    pub len: usize,
}

/// The letter groups a list of display names falls into, in the order
/// the list is already in.
///
/// A break is cut wherever [`key`] CHANGES from one name to the next,
/// which makes every group a contiguous run by construction — the grid
/// never reorders what the scanner sorted, and cannot draw a heading
/// over tiles that are not under it.
///
/// The consequence, said out loud: a letter appears twice if the list
/// is not sorted by that letter. [`crate::desktop::scan`] sorts by the
/// lowercased name, which is code-point order, and code-point order
/// puts every Latin letter with a diacritic AFTER `z` — so a menu
/// holding `Łoś` indexes `A`..`Z`, then `Ł`. That is the ORDERING being
/// visible, not the grouping being wrong, and the place to fix it is
/// the scanner's comparison, where a real collation would go. An index
/// that quietly re-sorted its own input to hide it would be lying about
/// the page under it.
pub fn of<'a, I: IntoIterator<Item = &'a str>>(names: I) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for (i, name) in names.into_iter().enumerate() {
        let k = key(name);
        match out.last_mut() {
            Some(s) if s.key == k => s.len += 1,
            _ => out.push(Section { key: k, start: i, len: 1 }),
        }
    }
    out
}

// ------------------------------------------------------------------ theme

/// Token ids one heading draws from, resolved by NAME once per epoch.
///
/// The three of them are ONE pattern the theme already has, rather than
/// three separate picks: `[table]` declares `head_role`, `rule` and — in
/// `component.table.rule` — the rule's ink, and a heading with a
/// hairline under it is exactly the shape of an alphabetical break. The
/// master ships `head_role = label.section`, "an instrument label above
/// a group", which is what a letter over its applications is, word for
/// word — but that is the THEME's sentence and this file reads it rather
/// than repeating it. It used to repeat it, which meant the trio was
/// really a pair plus a copy: two of the three moved with the theme and
/// the type did not.
///
/// The gap between the letter and its rule is `space.2` — `1u`, the
/// step of the ladder nearest the five device pixels the panel wants at
/// 1080 lines, `space.1` being `0.5u` and half of it. A number would
/// have been the one thing forbidden here.
///
/// What the theme has NO token for is a launcher's own section rule:
/// the right answer is a `component.launcher.section_*` trio beside
/// `component.launcher.tile`, and adding it is host-side work.
pub struct HeadTheme {
    pub epoch: u32,
    // type.<table.head_role>.* — the role the master BINDS a heading
    // to. Read as a word: which role a heading is set in is the theme's
    // decision, and a file that spells `type.label.section.*` out has
    // taken that decision back off it.
    size: u32,
    min: u32,
    tracking: u32,
    leading: u32,
    case: u32,
    fg: u32,
    /// The slot that role's `face` names, read WITH the ids because a
    /// face is a word and reading words is init-time work.
    font: u32,
    gap: u32,        // space.2 — letter to rule
    rule: u32,       // table.rule — the hairline's width
    rule_color: u32, // component.table.rule — its ink
}

impl HeadTheme {
    pub fn resolve(api: &HostApi, ctx: *mut c_void, epoch: u32) -> HeadTheme {
        // The BINDING, followed to the role it names — not the role
        // spelled out. `table.head_role` is the third member of the trio
        // the two lines below already take (`table.rule` and
        // `component.table.rule`), and taking two of three was the whole
        // defect: a theme moving its headings off `label.section` moved
        // every table in the program and left this index behind, at the
        // size of a role it never named.
        let head = tile::enum_word(api, ctx, tile::token(api, "table.head_role"));
        HeadTheme {
            epoch,
            size: tile::role_id(api, &head, "size"),
            min: tile::role_id(api, &head, "min_px"),
            tracking: tile::role_id(api, &head, "tracking"),
            leading: tile::role_id(api, &head, "leading"),
            case: tile::role_id(api, &head, "case"),
            fg: tile::role_id(api, &head, "fg"),
            font: tile::face_slot(api, ctx, tile::role_id(api, &head, "face")),
            gap: tile::token(api, "space.2"),
            rule: tile::token(api, "table.rule"),
            rule_color: tile::token(api, "component.table.rule"),
        }
    }
}

/// The heading values one frame draws with, read fresh from the
/// resolved ids. Colours and lengths only — nothing here is arithmetic
/// on anything.
pub struct HeadLook {
    pub px: f32,
    pub tracking: f32,
    pub leading: f32,
    pub case: u32,
    pub font: u32,
    pub ink: ColorC,
    /// `space.2`, between the letter's line box and the rule.
    pub gap: f32,
    pub rule_w: f32,
    pub rule: ColorC,
}

impl HeadLook {
    /// The pre-token world: a host that answers no theme calls at all.
    /// No ink and zero lengths — type of no size and a rule of no width
    /// draw nothing, which is the honest answer where the theme cannot
    /// be reached at all.
    pub fn raw() -> HeadLook {
        HeadLook {
            px: 0.0,
            tracking: 0.0,
            leading: 1.0,
            case: 0,
            font: tile::FONT_UI,
            ink: tile::NO_COLOR,
            gap: 0.0,
            rule_w: 0.0,
            rule: tile::NO_COLOR,
        }
    }

    pub fn read(api: &HostApi, ctx: *mut c_void, t: &HeadTheme) -> HeadLook {
        let px = |id| (api.theme_px)(ctx, id);
        HeadLook {
            px: px(t.size).max(px(t.min)),
            tracking: px(t.tracking),
            leading: px(t.leading).max(1.0),
            case: (api.theme_enum)(ctx, t.case),
            font: t.font,
            ink: (api.theme_color)(ctx, t.fg),
            gap: px(t.gap).max(0.0),
            // `table.rule` is declared `str | none`, and a theme that
            // says `none` means no rule. Whatever sentinel that bakes
            // to, a width at or below zero draws nothing and takes no
            // room, rather than reserving a negative band.
            rule_w: px(t.rule).max(0.0),
            rule: (api.theme_color)(ctx, t.rule_color),
        }
    }

    /// How tall one heading band is: the letter's line box, the gap,
    /// the rule, and then the grid's own `filetile.gap` before the
    /// first row of tiles under it.
    ///
    /// The last term is the tile grid's gap and not a fourth token,
    /// because below the rule the heading is over and what follows is
    /// the grid — and the distance between two of the grid's things is
    /// what `filetile.gap` IS. It is also what every row band already
    /// carries as its trailing space, so a heading and a row separate
    /// their neighbours by the same amount.
    pub fn height(&self, grid_gap: f32) -> f32 {
        (self.px * self.leading + self.gap + self.rule_w + grid_gap).max(0.0)
    }
}

/// One heading: the letter at the content box's left edge, and the rule
/// under it across the whole width.
///
/// `y` is the top of the band. The order down the page is the one the
/// panel asks for — letter, rule, then the applications the next bands
/// draw.
pub fn head(api: &HostApi, ctx: *mut c_void, look: &HeadLook, area: Rect, y: f32, key: char) {
    let sp = look.px * look.tracking;
    let text = tile::recase(look.case, key.to_string());
    if look.px > 0.0 {
        tile::text(api, ctx, look.font, look.px, area.x, y, &text, look.ink, sp, 0);
    }
    if look.rule_w > 0.0 && look.rule.a > 0.0 {
        let ry = y + look.px * look.leading + look.gap;
        (api.rect)(ctx, RectC { x: area.x, y: ry, w: area.w, h: look.rule_w }, look.rule);
    }
}

// ----------------------------------------------------------------- layout

/// One horizontal band of the indexed column: either a heading or a row
/// of tiles. The whole page is a list of these, in order, and scrolling
/// is choosing which of them is first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    /// The letter, when this band is a heading; None when it is tiles.
    pub key: Option<char>,
    /// A row's first tile, as a place in the list [`plan`] was given.
    pub first: usize,
    /// How many tiles are in the row. Zero for a heading.
    pub count: usize,
    /// The band's height, and its top measured from the column's.
    pub h: f32,
    pub y: f32,
}

/// An indexed page: the tile size the bands were built for, the bands
/// themselves, and which of them are on screen.
pub struct Plan {
    pub tile: f32,
    /// `filetile.gap`, as the bands were built with it.
    pub gap: f32,
    pub cols: usize,
    pub bands: Vec<Band>,
    /// The first band on screen.
    pub off: usize,
    /// How many bands fit from [`Plan::off`] down. Bands are NOT all
    /// the same height, so this is counted rather than divided.
    pub nvis: usize,
    /// The furthest first-band this column can be scrolled to.
    pub max_off: usize,
    /// The same bottom in the pixels the caller keeps its offset in —
    /// the y of that furthest band, remembered rather than recomputed
    /// so a dragged thumb and the clamp below cannot disagree. Bands
    /// differ in height, so this is NOT a count times a pitch.
    pub max_px: f32,
}

/// The bands `secs` make in `area`, and the scroll offset clamped to
/// them.
///
/// Scrolling snaps to whole bands for the reason the flat grid snaps to
/// whole rows: a heading half off the top is a letter that labels
/// nothing. `scroll` is the caller's own state and is corrected here,
/// because the bounds are arithmetic only this function does.
pub fn plan(
    look: &TileLook,
    head_h: f32,
    area: Rect,
    secs: &[Section],
    scroll: &mut f32,
) -> Plan {
    let (tile, cols) = tile::cells(look, area);
    let row_h = tile + look.gap;
    let mut bands: Vec<Band> = Vec::new();
    let mut y = 0.0f32;
    for s in secs {
        bands.push(Band { key: Some(s.key), first: s.start, count: 0, h: head_h, y });
        y += head_h;
        let mut done = 0usize;
        while done < s.len {
            let count = (s.len - done).min(cols.max(1));
            bands.push(Band { key: None, first: s.start + done, count, h: row_h, y });
            y += row_h;
            done += count;
        }
    }
    let total_h = y;

    // The last band that may be first: from it, everything left fits.
    // A band's trailing gap is allowed to hang past the floor, exactly
    // as the flat grid's `(area.h + gap) / row_h` allows the last row's.
    let room = area.h + look.gap;
    let max_off = bands
        .iter()
        .position(|b| total_h - b.y <= room)
        .unwrap_or_else(|| bands.len().saturating_sub(1));
    let max_px = bands.get(max_off).map(|b| b.y).unwrap_or(0.0).max(0.0);
    *scroll = scroll.clamp(0.0, max_px);

    // Which band the pixel offset has landed on: the nearest one, so a
    // notch that stops inside a band rounds rather than leaving it half
    // off the top.
    let off = bands
        .iter()
        .take(max_off + 1)
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.y - *scroll).abs().total_cmp(&(b.y - *scroll).abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);

    let top = bands.get(off).map(|b| b.y).unwrap_or(0.0);
    let mut nvis = 0usize;
    for b in bands.iter().skip(off) {
        if b.y + b.h - top > room {
            break;
        }
        nvis += 1;
    }
    // A panel too short for even one band still draws one: a page that
    // shows nothing at all is worse than a band clipped by its own
    // container.
    let nvis = nvis.max(1).min(bands.len().saturating_sub(off));

    Plan { tile, gap: look.gap, cols, bands, off, nvis, max_off, max_px }
}

impl Plan {
    /// This column, as the scrollbar reads it — bands where the flat
    /// grid counts rows, which is the same statement about the same
    /// eye: how much there is, how much is on show, where it is.
    pub fn scroll(&self) -> tile::Scroll {
        tile::Scroll {
            total: self.bands.len(),
            nvis: self.nvis,
            off: self.off,
            // Read off the band, never counted: the bands are NOT all
            // one height, so the top of the `off`-th is not `off` times
            // anything. A bar told the index instead would stand where
            // no hand could ever put it — a heading is ten pixels and a
            // row of tiles is twenty, and the thumb would leave the
            // finger dragging it by the difference.
            px: self.bands.get(self.off).map(|b| b.y).unwrap_or(0.0),
            max_px: self.max_px,
        }
    }

    /// The bands on screen, each with the y it is drawn at.
    pub fn visible(&self, area: Rect) -> impl Iterator<Item = (&Band, f32)> + '_ {
        let top = self.bands.get(self.off).map(|b| b.y).unwrap_or(0.0);
        let y0 = area.y;
        self.bands
            .iter()
            .skip(self.off)
            .take(self.nvis)
            .map(move |b| (b, y0 + b.y - top))
    }

    /// Where tile `n` of a row band sits — the same pitch across the
    /// row that [`tile::Layout::place`] uses, over the same tile edge
    /// and the same `filetile.gap`.
    pub fn cell(&self, area: Rect, n: usize, y: f32) -> Rect {
        Rect::new(area.x + n as f32 * (self.tile + self.gap), y, self.tile, self.tile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(secs: &[Section]) -> Vec<char> {
        secs.iter().map(|s| s.key).collect()
    }

    #[test]
    fn a_name_is_filed_under_its_own_letter_diacritics_included() {
        // The ordinary case, either case.
        assert_eq!(key("Firefox"), 'F');
        assert_eq!(key("gedit"), 'G');
        // Polish diacritics keep their own letters and are NOT folded
        // into the letters they resemble — three headings, not one bag.
        assert_eq!(key("Łoś"), 'Ł');
        assert_eq!(key("łoś"), 'Ł');
        assert_eq!(key("Żaba"), 'Ż');
        assert_eq!(key("żaba"), 'Ż');
        assert_eq!(key("Ósemka"), 'Ó');
        assert_eq!(key("ósemka"), 'Ó');
        assert_ne!(key("Łoś"), key("Los"));
        assert_ne!(key("Ósemka"), key("Osemka"));
        assert_ne!(key("Żaba"), key("Zaba"));
        // And they are not the fallback either.
        for n in ["Łoś", "Żaba", "Ósemka", "Ćma", "Ślimak", "Ąkacja", "Ęsik", "Ńowy"] {
            assert_ne!(key(n), NON_LETTER, "{n} is a letter");
        }
        // Other alphabets are letters too, and uppercase where the
        // script has a case at all.
        assert_eq!(key("Яндекс"), 'Я');
        assert_eq!(key("Δelta"), 'Δ');
        assert_eq!(key("字典"), '字');
        // A first letter that uppercases to several is filed under the
        // first of them, which is where a reader looks.
        assert_eq!(key("ßeta"), 'S');
        // Digits and everything else that is not a letter share one
        // group, so a menu does not grow a heading per digit.
        assert_eq!(key("0 A.D."), NON_LETTER);
        assert_eq!(key("2048"), NON_LETTER);
        assert_eq!(key("7-Zip"), NON_LETTER);
        assert_eq!(key("+Calc"), NON_LETTER);
        assert_eq!(key("_hidden"), NON_LETTER);
        assert_eq!(key(".config"), NON_LETTER);
        // A name that is not there at all still has somewhere to go.
        assert_eq!(key(""), NON_LETTER);
    }

    #[test]
    fn the_index_breaks_where_the_letter_changes_and_covers_everything() {
        let names = [
            "0 A.D.", "7-Zip", "Ark", "Audacity", "Blender", "Calc", "Łoś", "Żaba",
        ];
        let secs = of(names.iter().copied());
        assert_eq!(keys(&secs), ['#', 'A', 'B', 'C', 'Ł', 'Ż']);
        assert_eq!(secs[0], Section { key: '#', start: 0, len: 2 });
        assert_eq!(secs[1], Section { key: 'A', start: 2, len: 2 });
        assert_eq!(secs[5], Section { key: 'Ż', start: 7, len: 1 });
        // Every name is in exactly one group, and the groups are the
        // list in order with nothing added and nothing lost.
        assert_eq!(secs.iter().map(|s| s.len).sum::<usize>(), names.len());
        let mut at = 0;
        for s in &secs {
            assert_eq!(s.start, at, "the runs are contiguous");
            assert!(s.len > 0, "an empty group is never cut");
            at += s.len;
        }
        // Case does not split a group: the scanner sorts case-blind, so
        // `ark` beside `Ark` is one A and not two.
        assert_eq!(keys(&of(["Ark", "ark", "ARK"])), ['A']);
        // A single application is a single group; no applications is no
        // index at all, rather than one empty heading.
        assert_eq!(keys(&of(["Firefox"])), ['F']);
        assert!(of(std::iter::empty()).is_empty());
        // Unsorted input is grouped as it ARRIVES: the index never
        // reorders the page under it, so a letter seen twice is the
        // ordering showing through and not a lost group.
        assert_eq!(keys(&of(["Ark", "Blender", "Audacity"])), ['A', 'B', 'A']);
    }

    /// A look with the geometry the band arithmetic reads, and nothing
    /// else: tiles of 20 in columns of 2, gapless, so the numbers in
    /// the assertions below are the ones a reader can do in their head.
    fn look(area_w: f32) -> TileLook {
        let mut l = TileLook::raw();
        l.cell_pref = 20.0;
        l.cell_min = 1.0;
        l.cols = (area_w / 20.0).floor();
        l
    }

    #[test]
    fn the_column_is_a_heading_then_that_groups_rows_then_the_next() {
        // Five applications over two letters, two columns: A takes a
        // heading and two rows (3 tiles), B a heading and one row.
        let secs = of(["Ark", "Arena", "Audacity", "Blender", "Boxes"]);
        assert_eq!(keys(&secs), ['A', 'B']);
        let area = Rect::new(0.0, 0.0, 40.0, 1000.0);
        let mut s = 0.0;
        let p = plan(&look(40.0), 10.0, area, &secs, &mut s);
        assert_eq!(p.cols, 2);
        assert_eq!(p.tile, 20.0);
        let shape: Vec<(Option<char>, usize, usize)> =
            p.bands.iter().map(|b| (b.key, b.first, b.count)).collect();
        assert_eq!(shape, [
            (Some('A'), 0, 0),
            (None, 0, 2),
            (None, 2, 1),
            (Some('B'), 3, 0),
            (None, 3, 2),
        ]);
        // The tops stack: 10 for the heading, 20 per row.
        assert_eq!(p.bands.iter().map(|b| b.y).collect::<Vec<_>>(), [
            0.0, 10.0, 30.0, 50.0, 60.0
        ]);
        // Every tile of the list is placed exactly once, in order.
        let placed: Vec<usize> = p
            .bands
            .iter()
            .filter(|b| b.key.is_none())
            .flat_map(|b| b.first..b.first + b.count)
            .collect();
        assert_eq!(placed, [0, 1, 2, 3, 4]);
        // A tall panel holds the lot and does not scroll.
        assert_eq!((p.off, p.max_off, p.nvis), (0, 0, 5));
        assert_eq!(s, 0.0);
    }

    #[test]
    fn scrolling_an_indexed_column_snaps_to_whole_bands() {
        let secs = of(["Ark", "Arena", "Audacity", "Blender", "Boxes"]);
        // A box of 50: the heading (10) and two rows (20+20) fit, the
        // second heading does not.
        let area = Rect::new(0.0, 0.0, 40.0, 50.0);
        let mut s = 0.0;
        let p = plan(&look(40.0), 10.0, area, &secs, &mut s);
        assert_eq!((p.off, p.nvis), (0, 3));
        // From band 2 (y = 30) the rest is 80 - 30 = 50, which fits; from
        // band 1 (y = 10) it is 70, which does not. So band 2 is as far
        // as this column goes.
        assert_eq!(p.max_off, 2);
        // Scrolled past the end: the pixel figure is pulled back with
        // the offset, so the next notch is not swallowed undoing it.
        let mut s = 9999.0;
        let p = plan(&look(40.0), 10.0, area, &secs, &mut s);
        assert_eq!(p.off, 2);
        assert_eq!(s, 30.0);
        // A stop inside a band rounds to the nearer one.
        let mut s = 12.0;
        assert_eq!(plan(&look(40.0), 10.0, area, &secs, &mut s).off, 1);
        let mut s = 26.0;
        assert_eq!(plan(&look(40.0), 10.0, area, &secs, &mut s).off, 2);
        // The bands on screen are the ones the offset says, drawn from
        // the top of the content box down.
        let mut s = 30.0;
        let p = plan(&look(40.0), 10.0, area, &secs, &mut s);
        let seen: Vec<(Option<char>, f32)> =
            p.visible(area).map(|(b, y)| (b.key, y)).collect();
        assert_eq!(seen, [(None, 0.0), (Some('B'), 20.0), (None, 30.0)]);
        // A panel with no room for even one band still draws one.
        let tiny = Rect::new(0.0, 0.0, 40.0, 1.0);
        let p = plan(&look(40.0), 10.0, tiny, &secs, &mut 0.0);
        assert_eq!(p.nvis, 1);
        // And a menu with nothing in it is no bands at all, not a
        // division by zero.
        let p = plan(&look(40.0), 10.0, area, &[], &mut 0.0);
        assert!(p.bands.is_empty());
        assert_eq!((p.off, p.max_off), (0, 0));
        assert_eq!(p.scroll().total, 0);
    }

    /// The same geometry as [`look`], plus the bar a theme really asks
    /// for beside it: six wide, on the right, no margin, and no floor
    /// under the thumb — how short a thumb may get is a separate
    /// question from where it stands.
    fn look_with_bar(area_w: f32) -> TileLook {
        TileLook {
            sb_mode: tile::BarMode::Overlay,
            sb_w: 6.0,
            sb_margin: 0.0,
            sb_thumb_min: 0.0,
            sb_side: 0,
            ..look(area_w)
        }
    }

    /// The bands of an indexed column are NOT all one height, and that
    /// is the whole reason the bar is handed a pixel rather than the
    /// index beside it.
    ///
    /// A heading is ten tall here and a row of tiles twenty, so the
    /// second band's top is a THIRD of the way down everything this
    /// column can be scrolled through, while the second band is HALF the
    /// bands there are to stand on. A thumb placed by the second reading
    /// stands where no hand could have put it — grabbed and not moved it
    /// hands back an offset half again the one it was drawn from, so the
    /// page jumps on the first motion after the press and the thumb
    /// walks out from under the finger holding it.
    #[test]
    fn the_thumb_of_an_indexed_column_stands_where_the_hand_would_put_it() {
        let secs = of(["Ark", "Arena", "Audacity", "Blender", "Boxes"]);
        let area = Rect::new(0.0, 0.0, 40.0, 50.0);
        let look = look_with_bar(40.0);
        let mut s = 10.0;
        let p = plan(&look, 10.0, area, &secs, &mut s);
        // Bands at 0, 10 and 30 are the three this column may stand on,
        // so its bottom is thirty pixels and not two of anything.
        assert_eq!((p.off, p.max_off, p.max_px), (1, 2, 30.0));
        assert_eq!((p.scroll().px, p.scroll().max_px), (10.0, 30.0));
        // Three bands of five on show is a thumb 30 long in a track of
        // 50, leaving 20 of travel. A third of that travel is 6.67; the
        // index reading would have said half of it, 10.
        let bar = tile::bar_geom(&look, area, p.scroll()).expect("five bands through three");
        assert!((bar.thumb.y - 20.0 / 3.0).abs() < 0.01, "{}", bar.thumb.y);

        // And drawing the thumb and grabbing it are one arithmetic run
        // each way: the hand takes hold where the thumb stands, does not
        // move, and gets back the offset the thumb was drawn from.
        let mut g = tile::ThumbGrab::default();
        assert!(g.press(bar.thumb.y, &bar), "the thumb is where it was drawn");
        let back = g.drag_to(bar.thumb.y, &bar).expect("a held thumb answers");
        assert!((back - 10.0).abs() < 0.01, "{back}");
    }

    #[test]
    fn a_row_of_tiles_is_laid_out_across_before_it_is_laid_out_down() {
        let secs = of(["Ark", "Arena", "Audacity"]);
        let area = Rect::new(5.0, 7.0, 40.0, 1000.0);
        let p = plan(&look(40.0), 10.0, area, &secs, &mut 0.0);
        assert!(p.bands.iter().any(|b| b.key.is_none() && b.count == 2));
        let a = p.cell(area, 0, 100.0);
        let b = p.cell(area, 1, 100.0);
        assert_eq!((a.x, a.y, a.w, a.h), (5.0, 100.0, 20.0, 20.0));
        assert_eq!((b.x, b.y), (25.0, 100.0));
    }
}
