//! What the panel KNOWS: the query, the two source lists, the page they
//! rank into, and which row the keyboard is on.
//!
//! No drawing, no theme, no clock of its own — every method that needs
//! the time is given it — which is what makes the whole of "what does
//! this key do" testable without a window.
//!
//! # The field is not written here
//!
//! The query is a [`InputModel`] from the toolkit, and every key that
//! reaches it goes through the toolkit's own [`key_msg`]. That is the
//! point: caret motion by grapheme, word motion, undo grouping, the
//! selection, and an IME composition that must not enter the value
//! until it is committed are all decided in one place for the whole
//! project. A second answer to "what does ctrl+left mean" is a second
//! answer that will disagree.
//!
//! What this file adds is the two keys the FIELD does not own — up and
//! down walk the answer list — and the meaning of the two the field
//! answers with an intent: Enter runs what is chosen, Escape empties the
//! box.

use crate::rank::{self, Source};
use nacelle::focus::{Key, KeyEv};
use nacelle::object::text_input::{key_msg, InputEdited, InputModel, InputMsg};
use nacelle_launcher_core::desktop::AppEntry;
use std::path::PathBuf;

/// The longest page the panel will rank into. Behaviour, not look: a
/// theme decides how a row looks, and how many rows FIT is the panel's
/// own height — this is only the point past which ranking more answers
/// stops being work anybody reads.
pub const MAX_RESULTS: usize = 200;

/// The longest query the box will hold, in characters.
///
/// A search query has no business being longer, and the cap is load
/// bearing rather than tidy: the view records a caret position per
/// character every frame ([`crate::field`]), each measured from the
/// start of the value, so the drawing cost of a query grows with its
/// SQUARE. Unbounded, one pathological paste would be a frozen desktop.
pub const MAX_QUERY: usize = 256;

/// What one key meant. The caller redraws on anything but [`Ignored`],
/// and [`Ignored`] is also the answer that says "this key was not mine"
/// — the shape a host needs the day keys are routed to a focused widget
/// rather than to the on-screen keyboard alone.
///
/// [`Ignored`]: Outcome::Ignored
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Nothing here answers this key.
    Ignored,
    /// The caret, the selection or the chosen row moved. The query did
    /// not change, so nothing needs ranking again.
    Moved,
    /// The query text changed. The page has ALREADY been rebuilt from
    /// what is in memory; what waits for [`Finder::due`] is the walk of
    /// the home directory.
    Edited,
    /// Run this one.
    Activate(Source),
}

/// The throttle: how a burst of keystrokes becomes ONE walk of the home
/// directory.
///
/// It governs the DISK and nothing else. Ranking is a comparison against
/// two lists already in memory, so it happens on every keystroke and the
/// page is never a keystroke behind the box — which matters for more
/// than smoothness: a page that lagged the query would let Enter run an
/// answer to the question before this one.
///
/// Armed by every edit, and due only once the delay has passed since the
/// LAST of them — so typing `firefox` at any speed costs one walk rather
/// than seven. Time arrives as seconds from the caller's clock (the
/// host's `elapsed`), which is what lets this be tested with plain
/// numbers instead of by sleeping.
#[derive(Clone, Copy, Default, Debug)]
pub struct Throttle {
    armed: Option<f64>,
}

impl Throttle {
    /// An edit happened. Re-arming moves the deadline: the delay is
    /// measured from the last keystroke, never from the first.
    pub fn touch(&mut self, now: f64) {
        self.armed = Some(now);
    }

    /// Whether a search is waiting to run.
    pub fn armed(&self) -> bool {
        self.armed.is_some()
    }

    /// True exactly once per burst, when `delay` seconds have passed
    /// since the last [`Throttle::touch`]. A delay of zero fires on the
    /// next call, which is what a theme asking for no throttle means.
    pub fn due(&mut self, now: f64, delay: f32) -> bool {
        match self.armed {
            Some(t) if now - t >= delay as f64 => {
                self.armed = None;
                true
            }
            _ => false,
        }
    }
}

/// The panel's model.
pub struct Finder {
    /// The query field's state — the toolkit's, not this crate's.
    pub input: InputModel,
    apps: Vec<AppEntry>,
    files: Vec<PathBuf>,
    /// The ranked page: what is drawn, in the order it is drawn.
    hits: Vec<Source>,
    /// Which row the keyboard is on, as an index into `hits`.
    cursor: Option<usize>,
    throttle: Throttle,
}

impl Finder {
    pub fn new(apps: Vec<AppEntry>) -> Finder {
        Finder {
            input: InputModel::new().with_max_len(MAX_QUERY),
            apps,
            files: Vec::new(),
            hits: Vec::new(),
            cursor: None,
            throttle: Throttle::default(),
        }
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn hits(&self) -> &[Source] {
        &self.hits
    }

    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// What Enter would run.
    pub fn chosen(&self) -> Option<Source> {
        self.cursor.and_then(|i| self.hits.get(i)).copied()
    }

    /// The menu was rescanned. The page is rebuilt at once rather than
    /// on the next keystroke: an application installed while a query is
    /// on screen is an answer that was true a second ago.
    pub fn set_apps(&mut self, apps: Vec<AppEntry>) {
        // The identity is read BEFORE the list is replaced, or it would
        // be read out of the new list with an index into the old one.
        let was = self.chosen().and_then(|s| self.ident(s));
        self.apps = apps;
        self.rerank_keeping(was);
    }

    /// The home walk answered.
    pub fn set_files(&mut self, files: Vec<PathBuf>) {
        let was = self.chosen().and_then(|s| self.ident(s));
        self.files = files;
        self.rerank_keeping(was);
    }

    /// What a source IS, independently of where it currently sits: a
    /// desktop entry's id, or a file's path.
    ///
    /// A [`Source`] is an index, and an index means nothing across a
    /// rebuild of the list it points into — the walk answering replaces
    /// every file, so `File(3)` before and `File(3)` after are two
    /// different files that would silently swap under the cursor.
    fn ident(&self, s: Source) -> Option<String> {
        match s {
            Source::App(i) => self.apps.get(i).map(|a| a.id.clone()),
            Source::File(i) => self.files.get(i).map(|p| p.display().to_string()),
        }
    }

    /// Rebuilds the page from the two sources and the query.
    ///
    /// Cheap on purpose, and run on every keystroke: it is a grading of
    /// two lists already in memory. What is expensive is the walk that
    /// FILLS one of them, and that is what the throttle holds back.
    ///
    /// The chosen row is kept BY IDENTITY, not by index: the walk
    /// finishing under a settled query rebuilds the page around whatever
    /// the arrows had already reached, and a row that is no longer there
    /// gives the choice back to the top rather than to whatever moved
    /// into its place.
    pub fn rerank(&mut self) {
        let was = self.chosen().and_then(|s| self.ident(s));
        self.rerank_keeping(was);
    }

    /// [`Finder::rerank`], for a caller that has already replaced one of
    /// the source lists and read the chosen row's identity out of the
    /// list it replaced.
    fn rerank_keeping(&mut self, was: Option<String>) {
        self.hits = rank::rank(&self.apps, &self.files, self.query(), MAX_RESULTS);
        let mut found = None;
        if let Some(id) = &was {
            for (i, &h) in self.hits.iter().enumerate() {
                if self.ident(h).as_deref() == Some(id.as_str()) {
                    found = Some(i);
                    break;
                }
            }
        }
        self.cursor = match found {
            Some(i) => Some(i),
            None if self.hits.is_empty() => None,
            None => Some(0),
        };
    }

    /// Chooses a row by identity — what a click means.
    pub fn choose(&mut self, s: Source) {
        self.cursor = self.hits.iter().position(|h| *h == s);
    }

    /// Moves the chosen row by `delta`, clamped at both ends.
    ///
    /// Clamped rather than wrapped, and the answer at either end is
    /// [`Outcome::Ignored`]: an arrow that changed nothing has not been
    /// consumed, so a host with a focus chain can take it as the request
    /// to leave the list that it is.
    pub fn step(&mut self, delta: isize) -> Outcome {
        if self.hits.is_empty() {
            return Outcome::Ignored;
        }
        let last = self.hits.len() as isize - 1;
        let next = match self.cursor {
            None if delta >= 0 => 0,
            None => last,
            Some(c) => (c as isize + delta).clamp(0, last),
        };
        if self.cursor == Some(next as usize) {
            return Outcome::Ignored;
        }
        self.cursor = Some(next as usize);
        Outcome::Moved
    }

    /// Empties the query and the page with it. What Escape means, and
    /// what a caller does when it hands the panel back.
    pub fn clear(&mut self) {
        self.input.set_value("");
        self.files.clear();
        self.hits.clear();
        self.cursor = None;
    }

    /// Whether a search is waiting to run.
    pub fn armed(&self) -> bool {
        self.throttle.armed()
    }

    /// True once per burst of typing, `delay` seconds after the last
    /// keystroke — see [`Throttle`].
    pub fn due(&mut self, now: f64, delay: f32) -> bool {
        self.throttle.due(now, delay)
    }

    /// One key.
    ///
    /// Two keys are the LIST's and everything else is the field's. The
    /// two are not routed by focus because a panel with one field and
    /// one list needs no focus chain to know that up and down mean the
    /// list: the field is one line, and a one-line field has nowhere
    /// vertical to go.
    pub fn key(&mut self, ev: &KeyEv, now: f64) -> Outcome {
        // …with one exception, and it is the reason the test exists: an
        // IME candidate window is walked with the same two keys, and it
        // owns them until the composition commits.
        if !self.input.has_preedit() {
            match ev.key {
                Key::Up => return self.step(-1),
                Key::Down => return self.step(1),
                _ => {}
            }
        }
        let Some(msg) = key_msg(ev) else { return Outcome::Ignored };
        self.apply(msg, now)
    }

    /// One field message, and what it means to the panel around the
    /// field. Public because the platform speaks messages too — an IME
    /// commit is an [`InputMsg::Insert`] that never was a key.
    pub fn apply(&mut self, msg: InputMsg, now: f64) -> Outcome {
        match self.input.apply(msg) {
            InputEdited::Edited => {
                // The page first, the disk later: a page a keystroke
                // behind the box is a page Enter would run the wrong row
                // from.
                self.rerank();
                self.throttle.touch(now);
                Outcome::Edited
            }
            InputEdited::Submit => match self.chosen() {
                Some(s) => Outcome::Activate(s),
                // Enter over nothing is not a key this panel consumed.
                None => Outcome::Ignored,
            },
            InputEdited::Cancel => {
                // Escape empties the box. An Escape on an ALREADY empty
                // box is not this panel's: it belongs to whatever put
                // the panel on screen, and swallowing it would leave the
                // user pressing it twice.
                if self.query().is_empty() {
                    return Outcome::Ignored;
                }
                self.clear();
                // The same path a keystroke takes, so the caller has one
                // place to cancel a walk in flight and one place to
                // rebuild the page.
                self.throttle.touch(now);
                Outcome::Edited
            }
            InputEdited::Moved => Outcome::Moved,
            // The clipboard does not cross the plugin boundary: the ABI
            // has no entry for it, and a plugin's own copy of the
            // toolkit's clipboard is not the desktop's. Answering the
            // intent with a shrug is honest; pretending to copy is not.
            InputEdited::CopyRequest { .. } | InputEdited::PasteRequest => Outcome::Ignored,
            InputEdited::Rejected | InputEdited::None => Outcome::Ignored,
        }
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nacelle::focus::Mods;

    fn app(name: &str) -> AppEntry {
        AppEntry {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.to_string(),
            exec: "/bin/true".to_string(),
            terminal: false,
            icon: String::new(),
            categories: Vec::new(),
        }
    }

    fn ev(key: Key) -> KeyEv {
        KeyEv { key, mods: Mods::NONE, repeat: false, text: None }
    }

    fn typed(f: &mut Finder, s: &str, now: f64) {
        for c in s.chars() {
            assert_eq!(f.key(&ev(Key::Char(c)), now), Outcome::Edited);
        }
    }

    fn menu() -> Finder {
        let mut f = Finder::new(vec![app("Files"), app("Firefox"), app("Profiler")]);
        f.set_files(vec![PathBuf::from("/home/u/notes/file.txt")]);
        f
    }

    #[test]
    fn typing_ranks_the_page_at_once_and_only_the_walk_waits() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        assert_eq!(f.query(), "fi");
        // The page is current with the box, keystroke for keystroke:
        // two prefixes, then the file, then the name that merely
        // contains it — the ranking's own order.
        assert_eq!(f.hits().len(), 4);
        assert_eq!(f.chosen(), Some(Source::App(0)));
        // What is held back is the DISK, and only that.
        assert!(f.armed());
    }

    #[test]
    fn the_arrows_walk_the_page_and_stop_at_its_ends() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        assert_eq!(f.cursor(), Some(0));
        assert_eq!(f.key(&ev(Key::Down), 0.0), Outcome::Moved);
        assert_eq!(f.cursor(), Some(1));
        assert_eq!(f.key(&ev(Key::Up), 0.0), Outcome::Moved);
        assert_eq!(f.cursor(), Some(0));
        // The top and the bottom hold, and the key is NOT consumed
        // there: an arrow that moved nothing is an arrow to pass on.
        assert_eq!(f.key(&ev(Key::Up), 0.0), Outcome::Ignored);
        assert_eq!(f.cursor(), Some(0));
        for _ in 0..10 {
            f.key(&ev(Key::Down), 0.0);
        }
        assert_eq!(f.cursor(), Some(f.hits().len() - 1));
        assert_eq!(f.key(&ev(Key::Down), 0.0), Outcome::Ignored);
        // Arrows over an empty page do nothing at all.
        f.clear();
        assert_eq!(f.key(&ev(Key::Down), 0.0), Outcome::Ignored);
        assert_eq!(f.cursor(), None);
    }

    #[test]
    fn enter_runs_the_chosen_row_and_nothing_when_there_is_none() {
        let mut f = menu();
        // Typed and Entered inside the throttle's window, which is the
        // case that used to run the PREVIOUS query's first answer: the
        // page has to be current with the box before Enter is read.
        typed(&mut f, "fire", 0.0);
        assert!(f.armed(), "the walk has not run and must not have to");
        assert_eq!(f.key(&ev(Key::Enter), 0.0), Outcome::Activate(Source::App(1)));
        // The page survives being run: Enter opens something, it does
        // not consume the answer.
        assert_eq!(f.query(), "fire");
        // A query nothing answers has nothing to run.
        typed(&mut f, "zzz", 0.0);
        assert!(f.hits().is_empty());
        assert_eq!(f.key(&ev(Key::Enter), 0.0), Outcome::Ignored);
    }

    #[test]
    fn escape_empties_the_query_and_then_belongs_to_the_caller() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        assert_eq!(f.key(&ev(Key::Escape), 1.0), Outcome::Edited);
        assert_eq!(f.query(), "");
        assert!(f.hits().is_empty() && f.cursor().is_none());
        // The files of the query that just went are gone with it; a
        // second Escape is not this panel's key.
        assert!(f.files().is_empty());
        assert_eq!(f.key(&ev(Key::Escape), 1.0), Outcome::Ignored);
    }

    #[test]
    fn the_field_keeps_its_own_keys() {
        let mut f = menu();
        typed(&mut f, "abc", 0.0);
        // Caret motion is the field's and says so: moved, not edited.
        assert_eq!(f.key(&ev(Key::Left), 0.0), Outcome::Moved);
        assert_eq!(f.input.cursor(), 2);
        assert_eq!(f.key(&ev(Key::Backspace), 0.0), Outcome::Edited);
        assert_eq!(f.query(), "ac");
        // Ctrl+A is the toolkit's select-all, not a key this panel
        // invented, and Tab is nobody's here.
        let ctrl_a = KeyEv {
            key: Key::Char('a'),
            mods: Mods::CTRL,
            repeat: false,
            text: None,
        };
        assert_eq!(f.key(&ctrl_a, 0.0), Outcome::Moved);
        assert_eq!(f.input.selected_text(), Some("ac"));
        assert_eq!(f.key(&ev(Key::Tab), 0.0), Outcome::Ignored);
    }

    #[test]
    fn a_composing_ime_keeps_the_arrows() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        f.apply(InputMsg::Preedit("ちょ".into(), None), 0.0);
        // The candidate window is walked with the same two keys; the
        // page must not move under it.
        let before = f.cursor();
        assert_eq!(f.key(&ev(Key::Down), 0.0), Outcome::Ignored);
        assert_eq!(f.cursor(), before);
        // And the composition is not the value: nothing was searched.
        assert_eq!(f.query(), "fi");
        // Escape ends the composition rather than the query.
        assert_eq!(f.key(&ev(Key::Escape), 0.0), Outcome::Moved);
        assert_eq!(f.query(), "fi");
        assert_eq!(f.key(&ev(Key::Down), 0.0), Outcome::Moved);
    }

    #[test]
    fn the_query_is_bounded_and_a_refused_edit_changes_nothing() {
        let mut f = menu();
        f.input.set_value(&"x".repeat(MAX_QUERY));
        assert_eq!(f.key(&ev(Key::Char('y')), 0.0), Outcome::Ignored);
        assert_eq!(f.query().chars().count(), MAX_QUERY);
        // The cap is in CHARACTERS, which is what the per-character
        // caret positions are counted in — not in bytes.
        let mut f = menu();
        f.input.set_value(&"ż".repeat(MAX_QUERY));
        assert_eq!(f.key(&ev(Key::Char('y')), 0.0), Outcome::Ignored);
        assert_eq!(f.query().chars().count(), MAX_QUERY);
    }

    #[test]
    fn a_burst_of_typing_is_one_search() {
        let mut t = Throttle::default();
        assert!(!t.armed());
        assert!(!t.due(0.0, 0.2), "nothing was typed");
        t.touch(1.00);
        assert!(!t.due(1.10, 0.2), "still typing");
        t.touch(1.10);
        assert!(!t.due(1.25, 0.2), "the delay runs from the LAST keystroke");
        assert!(t.due(1.31, 0.2));
        assert!(!t.due(9.99, 0.2), "and it fires once");
        // A theme that asks for no throttle gets none.
        t.touch(2.0);
        assert!(t.due(2.0, 0.0));
    }

    #[test]
    fn seven_keystrokes_arm_one_search() {
        let mut f = menu();
        // Seven letters typed over a fifth of a second, the delay a
        // twentieth: without the re-arming rule this would be seven.
        let mut now = 0.0;
        for c in "firefox".chars() {
            now += 0.03;
            f.key(&ev(Key::Char(c)), now);
            assert!(!f.due(now, 0.05), "a search must not start mid-word");
        }
        assert!(f.due(now + 0.06, 0.05));
        assert!(!f.due(now + 1.0, 0.05));
    }

    #[test]
    fn a_rebuilt_page_keeps_the_row_the_arrows_had_reached() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        f.step(1);
        let chosen = f.chosen().unwrap();
        // The walk answers under the same query: the page is rebuilt
        // around the row that was already chosen.
        f.set_files(vec![
            PathBuf::from("/home/u/a/file.txt"),
            PathBuf::from("/home/u/b/finance.ods"),
        ]);
        assert_eq!(f.chosen(), Some(chosen));
        // A source that is gone gives the choice back to the top rather
        // than to whatever moved into its place.
        f.set_apps(vec![app("Firefox")]);
        assert_eq!(f.cursor(), Some(0));
        assert_eq!(f.chosen(), Some(Source::App(0)));
    }

    #[test]
    fn a_replaced_file_list_does_not_swap_the_chosen_row_under_the_cursor() {
        // The case an index-keyed selection gets wrong: the chosen row
        // is `File(0)`, the walk answers with a DIFFERENT set in which
        // `File(0)` is another file entirely, and the one that was
        // chosen is still there under a new index.
        let mut f = Finder::new(Vec::new());
        f.set_files(vec![PathBuf::from("/home/u/fig.png")]);
        typed(&mut f, "fi", 0.0);
        assert_eq!(f.chosen(), Some(Source::File(0)));
        f.set_files(vec![
            PathBuf::from("/home/u/first.txt"),
            PathBuf::from("/home/u/fig.png"),
        ]);
        assert_eq!(f.chosen(), Some(Source::File(1)), "the same FILE, not the same index");
        // And when it is gone from the answer, the choice starts over.
        f.set_files(vec![PathBuf::from("/home/u/first.txt")]);
        assert_eq!(f.cursor(), Some(0));
        assert_eq!(f.chosen(), Some(Source::File(0)));
        f.set_files(Vec::new());
        assert_eq!(f.cursor(), None);
    }

    #[test]
    fn a_click_chooses_by_identity() {
        let mut f = menu();
        typed(&mut f, "fi", 0.0);
        f.rerank();
        let last = *f.hits().last().unwrap();
        f.choose(last);
        assert_eq!(f.chosen(), Some(last));
        // A row that is not on the page cannot be chosen.
        f.choose(Source::File(99));
        assert_eq!(f.chosen(), None);
    }
}
