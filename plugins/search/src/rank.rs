//! Which of the two sources answers a query, and in what order.
//!
//! Nothing here touches the filesystem, the theme or a window: it is the
//! arithmetic of "what did the user mean", and it is the half of this
//! widget worth testing. A ranking nobody can reproduce on paper is a
//! ranking nobody can fix.
//!
//! # The three grades
//!
//! A name either IS the query, or STARTS with it, or merely HAS it
//! somewhere. That order is the whole of the relevance model, and it is
//! deliberately the whole of it: fuzzy scoring invites a table of magic
//! weights that no one can defend a year later, and a launcher that
//! cannot explain why it put `gedit` above `editor` has stopped being a
//! tool.

use nacelle_launcher_core::desktop::AppEntry;
use std::path::{Path, PathBuf};

/// Where one result came from, and where in that source it sits.
///
/// An index rather than a copy, so the row that is drawn and the thing
/// that is opened cannot disagree — and the two lists are rebuilt often
/// enough (a rescan, a finished walk) that the pair is re-ranked on the
/// same breath as either changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Index into the installed applications.
    App(usize),
    /// Index into the files the home walk found.
    File(usize),
}

/// How well a name answers a query. The `Ord` derive IS the ranking:
/// exact before prefix, prefix before contains, which is the order the
/// variants are declared in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Grade {
    Exact,
    Prefix,
    Contains,
}

/// How `name` answers `query`, or None when it does not answer at all.
///
/// Case-insensitive both ways, through `to_lowercase` rather than the
/// ASCII one: a Polish or Turkish menu entry must answer the letters its
/// user actually typed. An empty query answers nothing — an empty search
/// box is not a search for everything, it is a search that has not
/// started.
pub fn grade(name: &str, query: &str) -> Option<Grade> {
    if query.is_empty() || name.is_empty() {
        return None;
    }
    let n = name.to_lowercase();
    let q = query.to_lowercase();
    if n == q {
        Some(Grade::Exact)
    } else if n.starts_with(&q) {
        Some(Grade::Prefix)
    } else if n.contains(&q) {
        Some(Grade::Contains)
    } else {
        None
    }
}

/// The name a path is searched BY: its last component.
///
/// A name this build cannot read as text answers the empty string and
/// therefore never matches. That is honest rather than clever: a widget
/// that cannot draw a name has no business claiming it found it.
pub fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Both sources, graded against one query and ordered into the page the
/// panel draws.
///
/// The order, in full:
///
/// 1. the grade — an exact answer from either source beats every prefix
///    from both, because the user typed a whole name and meant it;
/// 2. applications before files at the same grade — a typed word is a
///    program more often than a document, and the grade above has
///    already decided the cases where it is not;
/// 3. the name, lowercased, so equal answers arrive in reading order;
/// 4. the source itself, so two entries with one name never swap places
///    between two frames.
///
/// `limit` is the page, not the search: everything is graded, the tail
/// is dropped, and dropping it AFTER the sort is what makes the limit a
/// cut through the ranking rather than through the scan order.
pub fn rank(apps: &[AppEntry], files: &[PathBuf], query: &str, limit: usize) -> Vec<Source> {
    // (grade, source kind, name, source) — the sort key, spelled out, so
    // the comparator is the tuple's own and there is no hand-written
    // `cmp` to get wrong.
    let mut graded: Vec<(Grade, u8, String, usize)> = Vec::new();
    for (i, a) in apps.iter().enumerate() {
        if let Some(g) = grade(&a.name, query) {
            graded.push((g, 0, a.name.to_lowercase(), i));
        }
    }
    for (i, p) in files.iter().enumerate() {
        let name = file_name(p);
        if let Some(g) = grade(name, query) {
            graded.push((g, 1, name.to_lowercase(), i));
        }
    }
    graded.sort();
    graded.truncate(limit);
    graded
        .into_iter()
        .map(|(_, kind, _, i)| if kind == 0 { Source::App(i) } else { Source::File(i) })
        .collect()
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    fn names(apps: &[AppEntry], files: &[PathBuf], out: &[Source]) -> Vec<String> {
        out.iter()
            .map(|s| match *s {
                Source::App(i) => apps[i].name.clone(),
                Source::File(i) => file_name(&files[i]).to_string(),
            })
            .collect()
    }

    #[test]
    fn the_three_grades_are_the_whole_relevance_model() {
        assert_eq!(grade("Files", "files"), Some(Grade::Exact));
        assert_eq!(grade("Files", "fil"), Some(Grade::Prefix));
        assert_eq!(grade("My Files", "fil"), Some(Grade::Contains));
        assert_eq!(grade("Files", "zzz"), None);
        // An empty query is a search that has not started, not a search
        // for everything.
        assert_eq!(grade("Files", ""), None);
        assert_eq!(grade("", "f"), None);
        // Case folds both ways, and past ASCII: the menu entry and the
        // typed letters meet in the same case.
        assert_eq!(grade("Łoś", "łoś"), Some(Grade::Exact));
        assert_eq!(grade("ŻABA", "żab"), Some(Grade::Prefix));
        assert_eq!(grade("Ósemka", "SEM"), Some(Grade::Contains));
        // And the order the ranking rests on.
        assert!(Grade::Exact < Grade::Prefix && Grade::Prefix < Grade::Contains);
    }

    #[test]
    fn an_exact_answer_beats_a_prefix_and_a_prefix_beats_a_containing_name() {
        let apps = [app("Terminal Emulator"), app("Term"), app("Xterm")];
        let files: Vec<PathBuf> = Vec::new();
        let out = rank(&apps, &files, "term", 10);
        assert_eq!(names(&apps, &files, &out), ["Term", "Terminal Emulator", "Xterm"]);
    }

    #[test]
    fn a_file_that_answers_better_beats_an_application_that_answers_worse() {
        let apps = [app("Note Taker")];
        let files = vec![PathBuf::from("/home/u/notes/note")];
        // The file IS the word; the application only starts with it. The
        // grade decides before the source does.
        let out = rank(&apps, &files, "note", 10);
        assert_eq!(names(&apps, &files, &out), ["note", "Note Taker"]);
        // At the SAME grade the application goes first.
        let apps = [app("Notes")];
        let files = vec![PathBuf::from("/home/u/notes")];
        let out = rank(&apps, &files, "note", 10);
        assert_eq!(out, [Source::App(0), Source::File(0)]);
    }

    #[test]
    fn equal_answers_arrive_in_reading_order_and_never_swap_between_frames() {
        let apps = [app("beta"), app("Alpha"), app("alpha")];
        let files: Vec<PathBuf> = Vec::new();
        let out = rank(&apps, &files, "a", 10);
        // "alpha"/"Alpha" are both prefixes and share a lowercased name,
        // so the index breaks the tie — the same order every time.
        assert_eq!(out, [Source::App(1), Source::App(2), Source::App(0)]);
        assert_eq!(rank(&apps, &files, "a", 10), out, "ranking is a function, not a shuffle");
    }

    #[test]
    fn the_limit_cuts_through_the_ranking_and_not_through_the_scan() {
        let apps = [app("aaa contains x"), app("xy"), app("x")];
        let files: Vec<PathBuf> = Vec::new();
        // Two of three: the exact and the prefix, never the first two
        // the scanner happened to hand over.
        let out = rank(&apps, &files, "x", 2);
        assert_eq!(names(&apps, &files, &out), ["x", "xy"]);
        assert!(rank(&apps, &files, "x", 0).is_empty());
        // Nothing answers an empty query, whatever the limit.
        assert!(rank(&apps, &files, "", 10).is_empty());
    }

    #[test]
    fn a_file_is_searched_by_its_last_component_and_not_by_its_path() {
        let apps: [AppEntry; 0] = [];
        let files = vec![
            PathBuf::from("/home/u/report/index.txt"),
            PathBuf::from("/home/u/index/notes.txt"),
        ];
        let out = rank(&apps, &files, "index", 10);
        // The directory named `index` does not make its child a hit; the
        // file named `index.txt` does.
        assert_eq!(out, [Source::File(0)]);
        assert_eq!(file_name(Path::new("/home/u/x.txt")), "x.txt");
        assert_eq!(file_name(Path::new("/")), "");
    }
}
