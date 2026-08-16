//! What this panel's user asked of it: `<config>/addons/filesystem.ron`.
//!
//! A plain file and not a directory, per the arrangement of 2026-08-12:
//! a directory is what an addon with a SECOND settings file gets, and
//! this one has four keys about a single subject — what the panel lists
//! and where it starts.
//!
//! Nothing about the LOOK is here and nothing ever will be. Tile size,
//! corner, gap, scroll physics, the ink of a name: all of that is the
//! theme's, read by token, and a copy of any of it in a settings file
//! would be a second place to change one thing.
//!
//! What is left is the four questions a file browser cannot answer for
//! somebody else: whether entries whose name begins with a dot are
//! listed, whether directories are gathered above files, whether the
//! panel follows the terminal's working directory, and which directory
//! it opens in. Every default below is what this panel did before it
//! could be asked, so a machine with no settings file behaves exactly
//! as it did.

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
/// `addons/filesystem.ron`, so a user who did would otherwise edit a
/// file, see a panel that looks factory-fresh, and get no sign
/// connecting the two — the silent slide this whole arrangement exists
/// to refuse.
pub fn unread(origin: Origin) -> Option<&'static str> {
    match origin {
        Origin::Refused => Some(
            "filesystem: this host delivers no addon settings \u{2014} \
             addons/filesystem.ron, if you wrote one, is not being read",
        ),
        _ => None,
    }
}

/// The panel's settings, as the user's file states them.
///
/// `#[serde(default)]` on the container is the one line that makes a RON
/// document survivable: the format parses all or nothing, so without it
/// a file that sets `hidden` and nothing else would fail WHOLE and the
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
    /// The directory the panel opens in. Empty means `$HOME`; `~` is
    /// expanded, an absolute path is taken as it stands, and anything
    /// else is refused by [`Config::start`].
    ///
    /// Read when the panel is CREATED and never again: it says where
    /// browsing begins, and re-reading it into the current directory
    /// would pull the reader out of wherever they had got to.
    pub start: String,
    /// Whether the panel walks with the active terminal tab's working
    /// directory. Off leaves it wherever the last click put it, which is
    /// what somebody watching one directory while working in another
    /// wants.
    pub follow_shell: bool,
    /// Whether entries whose name begins with a dot are listed. This
    /// panel has always listed them — it is the desktop's own file
    /// browser, not a menu — so the default is the behaviour and not the
    /// convention.
    pub hidden: bool,
    /// Whether directories are gathered above files. Off sorts by name
    /// alone, which is the order a person reading a list of names
    /// expects when the kinds do not matter to them.
    pub directories_first: bool,
}

impl Default for Config {
    /// What the panel did before it could be asked.
    fn default() -> Config {
        Config {
            start: String::new(),
            follow_shell: true,
            hidden: true,
            directories_first: true,
        }
    }
}

impl Config {
    /// The directory the panel opens in.
    ///
    /// `/` is the last resort rather than the process's working
    /// directory, exactly as it was before this setting existed: a panel
    /// opening on whatever the desktop happened to be started from is a
    /// panel opening somewhere nobody chose.
    pub fn start(&self) -> PathBuf {
        let home = || {
            std::env::var("HOME")
                .ok()
                .filter(|h| !h.is_empty())
                .map(PathBuf::from)
                .filter(|h| h.is_absolute())
        };
        if self.start.is_empty() {
            return home().unwrap_or_else(|| PathBuf::from("/"));
        }
        // `~/projects` is what a person writes in a settings file.
        if let Some(rest) = self.start.strip_prefix('~') {
            let rest = rest.trim_start_matches('/');
            return match home() {
                Some(h) if rest.is_empty() => h,
                Some(h) => h.join(rest),
                None => PathBuf::from("/"),
            };
        }
        let p = PathBuf::from(&self.start);
        if p.is_absolute() {
            return p;
        }
        // Said out loud rather than quietly swapped: the whole point of
        // a settings file is that a value in it can be connected to what
        // the program does.
        eprintln!(
            "filesystem: start {:?} in addons/filesystem.ron is not an absolute path \
             \u{2014} opening the home directory instead",
            self.start
        );
        home().unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Whether a change from `was` to this one changes what the CURRENT
    /// directory lists, and therefore needs the entries read again.
    ///
    /// `start` is not in it by construction — see the field — and
    /// `follow_shell` is not either: it decides what the next frame
    /// does, not what the present listing contains.
    pub fn relists(&self, was: &Config) -> bool {
        self.hidden != was.hidden || self.directories_first != was.directories_first
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
    /// before it could be asked.
    #[test]
    fn the_defaults_are_the_behaviour_that_needed_no_file() {
        let c = Config::default();
        assert!(c.hidden, "this panel has always shown dot files");
        assert!(c.follow_shell, "and has always walked with the terminal");
        assert!(c.directories_first);
        // The unset start is the home directory, which is what `create`
        // resolved for itself before there was a key for it.
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() && PathBuf::from(&home).is_absolute() {
                assert_eq!(c.start(), PathBuf::from(home));
            }
        }
    }

    #[test]
    fn the_panel_opens_where_the_file_says_and_never_where_it_was_run() {
        let abs = Config { start: "/srv/data".into(), ..Config::default() };
        assert_eq!(abs.start(), PathBuf::from("/srv/data"));
        // A relative start is refused; the fallback is a place, not the
        // desktop's working directory.
        let rel = Config { start: "data".into(), ..Config::default() };
        assert!(rel.start().is_absolute());
        if let Some(home) = std::env::var("HOME").ok().filter(|h| h.starts_with('/')) {
            let tilde = Config { start: "~/notes".into(), ..Config::default() };
            assert_eq!(tilde.start(), PathBuf::from(home).join("notes"));
        }
    }

    /// Which changes cost a re-read of the directory and which do not.
    /// A listing rebuilt on every settings change would be harmless; one
    /// never rebuilt would leave the panel showing what the OLD rules
    /// said, which is the bug this answers.
    #[test]
    fn only_the_listing_rules_send_the_panel_back_to_the_directory() {
        let base = Config::default();
        assert!(!base.relists(&base));
        assert!(Config { hidden: false, ..base.clone() }.relists(&base));
        assert!(Config { directories_first: false, ..base.clone() }.relists(&base));
        // Neither of these changes what the current directory contains.
        assert!(!Config { follow_shell: false, ..base.clone() }.relists(&base));
        assert!(!Config { start: "/srv".into(), ..base.clone() }.relists(&base));
    }

    /// The whole path a setting travels, through the host's own reader:
    /// the name this panel asks under, the file that name resolves to,
    /// and what a document stating one key does to the three it does
    /// not.
    ///
    /// One test and not four: the settings roots, their cache and their
    /// epoch are process-wide by design, and separate `#[test]`s would
    /// race each other under the default harness.
    #[test]
    fn the_host_answers_from_addons_filesystem_ron() {
        use nacelle::assets::AssetRoots;
        use nacelle::settings::{self, Origin};

        let base =
            std::env::temp_dir().join(format!("nacelle-fs-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("addons")).unwrap();
        settings::install(AssetRoots::new(vec![base.clone()], base.clone()));

        // No file: the defaults, and no complaint anywhere. An empty
        // config directory is what a fresh install looks like.
        let (cfg, origin) = settings::load::<Config>("filesystem", "");
        assert_eq!(origin, Origin::Absent);
        assert_eq!(cfg, Config::default());
        assert!(settings::problems().is_empty());

        std::fs::write(base.join("addons/filesystem.ron"), "(hidden: false)").unwrap();
        settings::reload();
        let (cfg, origin) = settings::load::<Config>("filesystem", "");
        assert_eq!(origin, Origin::File);
        assert!(!cfg.hidden, "the key the user wrote");
        assert!(cfg.follow_shell, "the keys they did not");
        assert!(cfg.relists(&Config::default()));

        // A key from a later build costs nothing.
        std::fs::write(
            base.join("addons/filesystem.ron"),
            "(hidden: false, sort_by_size: true)",
        )
        .unwrap();
        settings::reload();
        assert!(!settings::load::<Config>("filesystem", "").0.hidden);

        // A document that does not parse ends in the defaults and never
        // in half a config — and the host holds the path, so the user is
        // told which file and where in it.
        std::fs::write(base.join("addons/filesystem.ron"), "(hidden: false").unwrap();
        settings::reload();
        let (cfg, origin) = settings::load::<Config>("filesystem", "");
        assert_eq!(origin, Origin::Malformed);
        assert_eq!(cfg, Config::default());
        assert_eq!(settings::problems().len(), 1);

        // And the panel WIRED to all that: a file edited under a running
        // desktop reaches the screen. This is the half a settings module
        // cannot test by itself — an epoch gate that never fires, or a
        // field nobody acts on, would pass every assertion above.
        let tree = base.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("visible"), b"x").unwrap();
        std::fs::write(tree.join(".invisible"), b"x").unwrap();
        let mut fs = crate::Filesystem::new(tree.clone(), Config::default());
        let listed = |fs: &crate::Filesystem| fs.entries.iter().any(|e| e.name == ".invisible");
        assert!(listed(&fs), "the panel starts on the defaults");

        std::fs::write(base.join("addons/filesystem.ron"), "(hidden: false)").unwrap();
        settings::reload();
        fs.settings();
        assert!(!listed(&fs), "the edited file never reached the listing");

        // And nothing is re-read while the epoch stands still: the same
        // call again, on a file changed behind the toolkit's back, is
        // the frame that must NOT touch the disk.
        std::fs::write(base.join("addons/filesystem.ron"), "(hidden: true)").unwrap();
        fs.settings();
        assert!(!listed(&fs), "a settings file was parsed without being reloaded");

        let _ = std::fs::remove_dir_all(&base);
    }
}
