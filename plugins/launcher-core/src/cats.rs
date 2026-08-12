//! What a menu's categories ARE: the arithmetic that turns a pile of
//! desktop entries into the handful of groups this machine actually
//! has, with nothing drawn and nothing guessed.
//!
//! The vocabulary is the Desktop Menu Specification's own **main
//! categories** — the thirteen registered names a menu may be built out
//! of. Everything else a `Categories=` line carries is an ADDITIONAL
//! category (`Qt`, `GTK`, `Player`, `TextEditor`) or a vendor extension
//! (`X-Fedora-...`), and those describe an application rather than
//! place it, so they are read and dropped. An entry that lands in none
//! of the thirteen is not lost: it goes to [`OTHER`], because a
//! launcher that silently hides an installed application is worse than
//! one with an untidy last group.
//!
//! Nothing here draws, so all of it is testable without a window —
//! which is the whole reason it is a module rather than a corner of the
//! draw path.
//!
//! It lives in the launcher's shared crate rather than in either
//! widget, and that is not where it started. Both widgets need it and
//! they need different halves: the list asks [`group`] what groups
//! exist and how big each is, the grid asks [`holds`] whether one
//! application belongs to the group the list pointed it at. Two copies
//! of [`MAIN`] would be two vocabularies that drift apart on the first
//! category the specification adds, so this joins [`crate::desktop`]
//! and [`crate::tile`] as a third thing the family shares rather than
//! duplicates.

use crate::desktop::AppEntry;

/// The Desktop Menu Specification's main categories, in the order the
/// specification's own table lists them. Membership is exact and
/// case-sensitive, as the spec spells them: `audio` is not `Audio`, and
/// an entry that writes it in lower case has written a category no
/// registry knows.
pub const MAIN: [&str; 13] = [
    "AudioVideo",
    "Audio",
    "Video",
    "Development",
    "Education",
    "Game",
    "Graphics",
    "Network",
    "Office",
    "Science",
    "Settings",
    "System",
    "Utility",
];

/// Where an entry with no main category goes. English in the code, as
/// every string in this tree is: what a user reads is the theme's and
/// the locale's business, not this file's.
pub const OTHER: &str = "Other";

/// One group, and which applications are in it. The numbers index the
/// entry list the group was computed from — a category owns no entries
/// of its own, so a rescan replaces the groups and never the menu.
#[derive(Clone, Debug)]
pub struct Category {
    pub name: &'static str,
    pub apps: Vec<usize>,
}

/// The main categories one `Categories=` value belongs to, in [`MAIN`]'s
/// order, each at most once.
///
/// An entry may name several — `AudioVideo;Audio;Player;` is a music
/// player, and the spec's own table says such an entry belongs in BOTH
/// the multimedia group and the audio one — so this answers a set
/// rather than a first match. The empty answer is the honest one for
/// `Qt;GTK;` and for a file with no `Categories=` at all.
pub fn main_categories(categories: &[String]) -> Vec<&'static str> {
    main_slots(categories).into_iter().map(|i| MAIN[i]).collect()
}

/// [`main_categories`] as indices into [`MAIN`] — what the grouping
/// itself wants, and what keeps the answer in the specification's order
/// without a second search per name.
fn main_slots(categories: &[String]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for c in categories {
        // Trimmed because a hand-written entry file may write
        // `Categories=Utility; Development;`, and the space is a typing
        // habit rather than a different category.
        let c = c.trim();
        if let Some(i) = MAIN.iter().position(|m| *m == c) {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out.sort_unstable();
    out
}

/// The groups this machine has: every main category that at least one
/// installed application claims, plus [`OTHER`] when anything claimed
/// none, sorted by name.
///
/// Sorted by NAME and not by the specification's order, because the
/// list is read rather than iterated: a reader looking for `Office`
/// looks where the alphabet says it is. `Other` takes its alphabetical
/// place with the rest — it is a group like any other here, not a
/// footnote, and pinning it to the end would be this file inventing a
/// rank that nothing asked for.
///
/// The counts are memberships, not applications: an entry in both
/// `AudioVideo` and `Audio` is counted by both groups, so the counts
/// sum to more than the menu. That is what the categories ARE, and a
/// count that hid it would be a different, wrong number.
pub fn group(entries: &[AppEntry]) -> Vec<Category> {
    let mut main: Vec<Vec<usize>> = vec![Vec::new(); MAIN.len()];
    let mut other: Vec<usize> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let slots = main_slots(&e.categories);
        if slots.is_empty() {
            other.push(i);
            continue;
        }
        for s in slots {
            main[s].push(i);
        }
    }
    let mut out: Vec<Category> = main
        .into_iter()
        .enumerate()
        .map(|(i, apps)| Category { name: MAIN[i], apps })
        .chain(std::iter::once(Category { name: OTHER, apps: other }))
        .filter(|c| !c.apps.is_empty())
        .collect();
    out.sort_by(|a, b| a.name.cmp(b.name));
    out
}

/// Whether one application belongs to the group called `name` — the
/// same question [`group`] answers for the whole menu at once, asked of
/// a single entry.
///
/// The grid needs it in this shape because it is handed a NAME and not
/// a group: the selection travels between two widgets as the name of a
/// category (see [`crate::selection`]), and the grid then has its own
/// scan to filter, not the list's. Both paths read [`main_slots`], so
/// "what is in Utility" cannot mean one thing in the list's count and
/// another in the grid's page.
///
/// [`OTHER`] is not a category any entry claims — it is the ABSENCE of
/// every main one — so it is answered by asking exactly that.
pub fn holds(name: &str, e: &AppEntry) -> bool {
    let slots = main_slots(&e.categories);
    if name == OTHER {
        slots.is_empty()
    } else {
        slots.iter().any(|&i| MAIN[i] == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry with nothing filled in but the two fields these tests
    /// look at. The scanner's own parsing has its own tests next door;
    /// what is checked here is only what happens to `Categories=`
    /// AFTER it has been split.
    fn entry(name: &str, categories: &[&str]) -> AppEntry {
        AppEntry {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.to_string(),
            exec: "/bin/true".to_string(),
            terminal: false,
            icon: String::new(),
            categories: categories.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn only_the_specifications_main_categories_survive() {
        let cats = |v: &[&str]| {
            main_categories(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        // The ordinary music player: two main categories on one entry,
        // and the additional ones dropped.
        assert_eq!(cats(&["AudioVideo", "Audio", "Player", "Qt", "KDE"]), [
            "AudioVideo",
            "Audio"
        ]);
        // Additional categories alone place nothing.
        assert!(cats(&["Qt", "GTK", "Player", "TextEditor"]).is_empty());
        assert!(cats(&["X-Fedora-Something", "X-GNOME-Utilities"]).is_empty());
        // No Categories= line at all.
        assert!(cats(&[]).is_empty());
        // Exact spelling: the registry is case-sensitive and knows no
        // plurals.
        assert!(cats(&["utility", "GAME", "Games", "Utilities"]).is_empty());
        // A category written twice is one membership.
        assert_eq!(cats(&["Utility", "Utility", "Utility"]), ["Utility"]);
        // The answer is in the specification's order whatever order the
        // file wrote them in, and a typed space is not a category.
        assert_eq!(cats(&["Utility", " Development ", "AudioVideo"]), [
            "AudioVideo",
            "Development",
            "Utility"
        ]);
        // Every one of the thirteen is recognised.
        for m in MAIN {
            assert_eq!(cats(&[m]), [m], "{m} is a main category");
        }
    }

    #[test]
    fn every_membership_is_counted_and_nothing_installed_is_lost() {
        let entries = [
            entry("Player", &["AudioVideo", "Audio", "Player"]),
            entry("Recorder", &["AudioVideo", "Audio"]),
            entry("Cutter", &["AudioVideo", "Video", "Qt"]),
            entry("Editor", &["Utility", "TextEditor"]),
            entry("Nothing", &[]),
            entry("Toolkit", &["Qt", "KDE"]),
        ];
        let got = group(&entries);
        let seen: Vec<(&str, usize)> =
            got.iter().map(|c| (c.name, c.apps.len())).collect();
        assert_eq!(seen, [
            ("Audio", 2),
            ("AudioVideo", 3),
            ("Other", 2),
            ("Utility", 1),
            ("Video", 1),
        ]);
        // A group holds indices into the list it was computed from, and
        // the entry named by one really is what it claims.
        let audio = got.iter().find(|c| c.name == "Audio").unwrap();
        let names: Vec<&str> =
            audio.apps.iter().map(|&i| entries[i].name.as_str()).collect();
        assert_eq!(names, ["Player", "Recorder"]);
        // Memberships, not applications: six entries, nine memberships.
        assert_eq!(got.iter().map(|c| c.apps.len()).sum::<usize>(), 9);
        // Every entry is reachable from some group — the launcher hides
        // nothing that is installed.
        let mut reached: Vec<usize> =
            got.iter().flat_map(|c| c.apps.iter().copied()).collect();
        reached.sort_unstable();
        reached.dedup();
        assert_eq!(reached.len(), entries.len());
    }

    #[test]
    fn the_list_is_alphabetical_and_holds_only_groups_that_exist() {
        // One entry in each of three categories, offered in an order
        // that is neither alphabetical nor the specification's.
        let entries = [
            entry("Term", &["System", "TerminalEmulator"]),
            entry("Doc", &["Office"]),
            entry("Ide", &["Development"]),
            entry("Wild", &["NotACategory"]),
        ];
        let names: Vec<&str> = group(&entries).iter().map(|c| c.name).collect();
        assert_eq!(names, ["Development", "Office", "Other", "System"]);
        // The ten main categories nothing claims are not shown at all.
        assert!(!names.contains(&"Game"));
        // A menu with nothing in it has no groups, not thirteen empty
        // ones.
        assert!(group(&[]).is_empty());
        // And a menu where everything is placed has no Other.
        let placed = [entry("Ide", &["Development"])];
        assert_eq!(group(&placed).iter().map(|c| c.name).collect::<Vec<_>>(), [
            "Development"
        ]);
    }

    #[test]
    fn one_entry_answers_the_same_membership_the_whole_grouping_does() {
        let entries = [
            entry("Player", &["AudioVideo", "Audio", "Player"]),
            entry("Cutter", &["AudioVideo", "Video", "Qt"]),
            entry("Editor", &["Utility", "TextEditor"]),
            entry("Nothing", &[]),
            entry("Toolkit", &["Qt", "KDE"]),
        ];
        // The filter the grid runs and the grouping the list draws are
        // the same answer, group by group — which is the only property
        // that matters, since one widget writes the name and the other
        // reads it.
        for c in group(&entries) {
            let by_holds: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| holds(c.name, e))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(by_holds, c.apps, "{} disagrees", c.name);
        }
        // Other is the absence of every main category, not a category
        // an entry can write.
        assert!(holds(OTHER, &entry("Nothing", &[])));
        assert!(holds(OTHER, &entry("Toolkit", &["Qt", "KDE"])));
        assert!(!holds(OTHER, &entry("Editor", &["Utility"])));
        assert!(holds(OTHER, &entry("Liar", &["Other"])), "`Other` is not main");
        // A name no grouping ever produced holds nothing — what the
        // grid sees when the last application of a group is
        // uninstalled while that group is the selected one.
        assert!(!holds("Nonesuch", &entry("Editor", &["Utility"])));
        assert!(!holds("", &entry("Editor", &["Utility"])));
    }
}
