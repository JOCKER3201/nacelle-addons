//! What this panel's user asked of it: `<config>/addons/search.ron`.
//!
//! A plain file and not a directory, per the arrangement of 2026-08-12:
//! a directory is what an addon with a SECOND settings file gets, and
//! this one has five keys about a single subject.
//!
//! # Why this panel has settings and the launcher does not
//!
//! Everything here is about the HOME WALK, which is the one thing this
//! widget does that touches the user's own filesystem — how far into it
//! it goes, how much of it it may look at, and whether it goes at all.
//! That is precisely the class of question a settings file is for, and
//! the class the theme was explicitly refused for (see [`crate::files`]:
//! a theme decides how a result LOOKS, never how much of somebody's
//! filesystem a widget may touch).
//!
//! The applications half has no settings and gets none. It shows what
//! the XDG specification says is installed, filtered by the rules that
//! specification states; a launcher answering with a different set from
//! every other launcher on the machine would not be configured, it would
//! be wrong. The look of all of it — every colour, length and duration,
//! the typing debounce among them — is the theme's, and none of it is
//! here.

use crate::files::{self, Limits};
use nacelle::settings::Origin;
use std::path::PathBuf;

/// What to say about a read whose outcome the TOOLKIT does not report,
/// or `None` when there is nothing to say — which is the usual answer.
///
/// A separate function from the printing so that it can be tested
/// without the process-wide settings roots, which no test can put back
/// the way it found them.
///
/// [`Origin::Refused`] under a name this file spells out — a plain,
/// static one the toolkit accepts — means one thing only: the host
/// never installed the settings directories, so no file was even looked
/// for. That is the one outcome nothing else mentions. A document that
/// will not parse is reported by the toolkit, with the path and the
/// position; an absent file is not an event; a file that read is the
/// point. But the toolkit cannot know that anybody WROTE
/// `addons/search.ron`, so a user who did would otherwise edit a file,
/// see a panel that looks factory-fresh, and get no sign connecting the
/// two — the silent slide this whole arrangement exists to refuse.
pub fn unread(origin: Origin) -> Option<&'static str> {
    match origin {
        Origin::Refused => Some(
            "search: this host delivers no addon settings \u{2014} \
             addons/search.ron, if you wrote one, is not being read",
        ),
        _ => None,
    }
}

/// The panel's settings, as the user's file states them.
///
/// `#[serde(default)]` on the container is the one line that makes a
/// RON document survivable: the format parses all or nothing, so without
/// it a file that sets `depth` and nothing else would fail WHOLE and the
/// user would lose the one key they wrote. With it, the document says
/// what it says and [`Config::default`] answers the rest.
///
/// There is deliberately no `deny_unknown_fields`. A key this build has
/// never heard of is a file written for a LATER one, and refusing the
/// whole document over it would turn a forward-compatible file into a
/// panel running on defaults.
#[derive(serde::Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Whether the home walk runs at all. `false` leaves the panel with
    /// its applications half, which is a whole search — and the only
    /// setting here that costs nothing to be sure of.
    pub files: bool,
    /// Where the walk starts. Empty means `$HOME`; `~` is expanded, an
    /// absolute path is taken as it stands, and anything else is refused
    /// by [`Config::root`].
    pub root: String,
    /// Directories below the root. 0 walks the root itself only.
    pub depth: u32,
    /// How many files one walk keeps before it stops.
    pub hits: usize,
    /// How many entries a walk may LOOK AT before it gives up on the
    /// rest — the limit that bounds the work rather than the answer.
    pub visited: usize,
}

impl Default for Config {
    /// What the panel did before it could be asked: the walk on, rooted
    /// at the home directory, inside [`Limits::default`].
    ///
    /// The three numbers are read OFF that type rather than written
    /// again here, so the shipped behaviour and the documented default
    /// cannot drift apart.
    fn default() -> Config {
        let lim = Limits::default();
        Config {
            files: true,
            root: String::new(),
            depth: lim.depth,
            hits: lim.hits,
            visited: lim.visited,
        }
    }
}

impl Config {
    /// The bounds one walk runs under.
    pub fn limits(&self) -> Limits {
        Limits { depth: self.depth, hits: self.hits, visited: self.visited }
    }

    /// The directory a walk starts in, or `None` when there is to be no
    /// walk — `files: false`, or a machine whose `$HOME` this build will
    /// not guess at ([`files::home`]).
    ///
    /// Called when the settings are READ and not once per query: a root
    /// that cannot be used is reported here, and once per settings file
    /// is the right number of times to say it.
    pub fn root(&self) -> Option<PathBuf> {
        if !self.files {
            return None;
        }
        if self.root.is_empty() {
            return files::home();
        }
        // `~/projects` is what a person writes in a settings file, and a
        // panel that walked `$HOME` instead while saying nothing would
        // be answering a question nobody asked.
        if let Some(rest) = self.root.strip_prefix('~') {
            let rest = rest.trim_start_matches('/');
            return files::home().map(|h| if rest.is_empty() { h } else { h.join(rest) });
        }
        let p = PathBuf::from(&self.root);
        if p.is_absolute() {
            return Some(p);
        }
        // A relative root would be resolved against the desktop
        // process's working directory — whatever it happened to be
        // started from, which is the same reason `files::home` refuses a
        // relative `$HOME`. Said out loud rather than quietly swapped:
        // the whole point of the settings file is that a value in it can
        // be connected to what the panel does.
        eprintln!(
            "search: root {:?} in addons/search.ron is not an absolute path \
             \u{2014} walking the home directory instead",
            self.root
        );
        files::home()
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Which outcomes this panel says something about, and which it
    /// leaves to somebody who says it better.
    ///
    /// Free of the process-wide roots on purpose: the point is the
    /// DECISION, and a test that had to leave the settings module
    /// uninstalled could not run beside the one below.
    #[test]
    fn a_host_that_reads_no_settings_at_all_is_the_one_thing_said_here() {
        assert!(unread(Origin::Refused).is_some());
        // The toolkit reports this one, with the path and the position.
        // A second copy of that diagnostic is noise.
        assert!(unread(Origin::Malformed).is_none());
        // Neither of these is an event: a fresh install has no file, and
        // a file that read is the whole point.
        assert!(unread(Origin::Absent).is_none());
        assert!(unread(Origin::File).is_none());
    }

    /// A settings file nobody wrote leaves the panel exactly as it was
    /// before it could be asked. This is the test that fails if a
    /// default is ever changed by editing this file alone.
    #[test]
    fn the_defaults_are_the_behaviour_that_needed_no_file() {
        let c = Config::default();
        assert!(c.files, "the walk was on before there was a key for it");
        assert_eq!(c.limits(), Limits::default());
        assert_eq!(c.root(), files::home());
    }

    /// The whole path a setting travels, through the host's own reader
    /// rather than through a parser called here: the name this panel
    /// asks under, the file that name resolves to, and what a document
    /// stating one key does to the four it does not.
    ///
    /// One test and not four: the settings roots, their cache and their
    /// epoch are process-wide by design, and separate `#[test]`s would
    /// race each other under the default harness.
    #[test]
    fn the_host_answers_from_addons_search_ron() {
        use nacelle::assets::AssetRoots;
        use nacelle::settings::{self, Origin};

        let base =
            std::env::temp_dir().join(format!("nacelle-search-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("addons")).unwrap();
        settings::install(AssetRoots::new(vec![base.clone()], base.clone()));

        // No file: the defaults, and no complaint anywhere. An empty
        // config directory is what a fresh install looks like.
        let (cfg, origin) = settings::load::<Config>("search", "");
        assert_eq!(origin, Origin::Absent);
        assert_eq!(cfg, Config::default());
        assert!(settings::problems().is_empty());

        // The name this panel asks under is the file the user edits.
        // A directory member (`search/engines.ron`) is deliberately NOT
        // used: this addon has one subject and therefore one file.
        std::fs::write(base.join("addons/search.ron"), "(depth: 2)").unwrap();
        settings::reload();
        let (cfg, origin) = settings::load::<Config>("search", "");
        assert_eq!(origin, Origin::File);
        assert_eq!(cfg.depth, 2, "the key the user wrote");
        assert_eq!(cfg.hits, Config::default().hits, "the keys they did not");
        assert!(cfg.files);

        // A key from a later build costs nothing: the document is not
        // refused over a name this one has never heard of.
        std::fs::write(
            base.join("addons/search.ron"),
            "(depth: 3, follow_symlinks: true)",
        )
        .unwrap();
        settings::reload();
        assert_eq!(settings::load::<Config>("search", "").0.depth, 3);

        // A document that does not parse ends in the defaults and NEVER
        // in half a config — and the host has the path, so the user is
        // told which file and where in it.
        std::fs::write(base.join("addons/search.ron"), "(depth: 3").unwrap();
        settings::reload();
        let (cfg, origin) = settings::load::<Config>("search", "");
        assert_eq!(origin, Origin::Malformed);
        assert_eq!(cfg, Config::default());
        assert_eq!(settings::problems().len(), 1);

        // And the panel WIRED to all that: a file edited under a running
        // desktop reaches the walk. This is the half a settings module
        // cannot test by itself — an epoch gate that never fires, or a
        // field nobody acts on, would pass every assertion above.
        std::fs::write(base.join("addons/search.ron"), "(files: false)").unwrap();
        settings::reload();
        let mut panel = crate::Search::new();
        assert_eq!(panel.root, None, "the panel starts on the file it was given");

        std::fs::write(base.join("addons/search.ron"), "(root: \"/srv/data\")").unwrap();
        settings::reload();
        panel.settings();
        assert_eq!(
            panel.root,
            Some(PathBuf::from("/srv/data")),
            "the edited file never reached the walk"
        );
        assert_eq!(panel.cfg.limits(), Limits::default(), "a key nobody set moved");

        // And nothing is re-read while the epoch stands still: the same
        // call again, on a file changed behind the toolkit's back, is
        // the frame that must NOT touch the disk.
        std::fs::write(base.join("addons/search.ron"), "(root: \"/srv/other\")").unwrap();
        panel.settings();
        assert_eq!(panel.root, Some(PathBuf::from("/srv/data")));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_walk_is_rooted_where_the_file_says_and_nowhere_else() {
        let abs = Config { root: "/data/notes".into(), ..Config::default() };
        assert_eq!(abs.root(), Some(PathBuf::from("/data/notes")));
        // Off means off: there is no root, so nothing starts a walk.
        let off = Config { files: false, root: "/data".into(), ..Config::default() };
        assert_eq!(off.root(), None);
        // A relative root is refused and the home directory answers, so
        // the panel never walks the desktop's working directory.
        let rel = Config { root: "notes".into(), ..Config::default() };
        assert_eq!(rel.root(), files::home());
        // `~` is a place a person writes, and it means the home.
        if let Some(home) = files::home() {
            let tilde = Config { root: "~/notes".into(), ..Config::default() };
            assert_eq!(tilde.root(), Some(home.join("notes")));
            let bare = Config { root: "~".into(), ..Config::default() };
            assert_eq!(bare.root(), Some(home));
        }
    }
}
