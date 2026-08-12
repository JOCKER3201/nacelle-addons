//! The second source: files under the home directory.
//!
//! A desktop search that walks a home directory on the thread that draws
//! is a desktop that stops for as long as the walk takes, and a home
//! directory is exactly the place where "as long as the walk takes" is
//! unbounded — a checkout, a mail spool, a node_modules. So the walk runs
//! on a thread of its own ([`Scan`]), reports through a channel, and is
//! bounded three ways at once ([`Limits`]): how deep it goes, how many
//! answers it keeps, and how many entries it is allowed to look at
//! before it gives up on the rest.
//!
//! The limits are ENGINE constants and not theme tokens, for the same
//! reason `UNDO_DEPTH` is one: a theme decides how a result looks, never
//! how much of somebody's filesystem a widget may touch.
//!
//! # What the walk refuses
//!
//! * anything whose name starts with a dot — `.cache`, `.git`, `.local`,
//!   a stray `.DS_Store`. Hidden means "not for a menu" everywhere else
//!   in this project (a `NoDisplay=true` desktop entry), and a search
//!   answering with cache files is a search nobody reads twice.
//! * symlinks, in both directions: a link is not a place. The depth cap
//!   alone would survive a loop, but it would survive it by walking the
//!   same tree eight times, and a link's target is reachable under its
//!   own path anyway.
//! * a name this build cannot read as text ([`crate::rank::file_name`]).

use crate::rank::grade;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

/// How far a walk may go before it stops, whatever it has found.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Directories below the root. 0 walks the root itself only.
    pub depth: u32,
    /// Answers kept. The walk ends the moment it has this many.
    pub hits: usize,
    /// Entries LOOKED AT — the one limit that bounds the work rather
    /// than the answer, and therefore the one that keeps a query nobody
    /// matches from costing as much as a `find /`.
    pub visited: usize,
}

impl Default for Limits {
    /// Deep enough for `~/projects/thing/src/main.rs` and no deeper, a
    /// page of answers nobody will scroll past, and a fifth of a second
    /// of `readdir` on a warm cache.
    fn default() -> Limits {
        Limits { depth: 8, hits: 200, visited: 20_000 }
    }
}

/// Whether the walk refuses an entry by its name alone. Hidden entries
/// are refused, `.cache` among them — spelled out in the test, because
/// the day the hidden rule loosens is the day `.cache` needs its own.
pub fn refused(name: &str) -> bool {
    name.is_empty() || name.starts_with('.')
}

/// The home directory, when the environment names an absolute one.
///
/// `$HOME` unset, empty or relative is not a home this widget will guess
/// at: a search rooted at the working directory of the desktop process
/// would answer with whatever it happened to be started from.
pub fn home() -> Option<PathBuf> {
    let h = std::env::var("HOME").ok().filter(|v| !v.is_empty())?;
    let p = PathBuf::from(h);
    p.is_absolute().then_some(p)
}

/// One walk of `root` for `query`, bounded by `lim` and abandoned as
/// soon as `cancel` is set.
///
/// Synchronous and pure but for the filesystem it reads, which is what
/// lets the limits be tested against a tree built in a temporary
/// directory rather than against somebody's actual home.
///
/// The order is breadth-agnostic on purpose: the caller RANKS what comes
/// back ([`crate::rank::rank`]), so the order files are found in never
/// reaches the screen, and a stack is cheaper than a queue.
pub fn walk(root: &Path, query: &str, lim: Limits, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if query.is_empty() {
        return out;
    }
    let mut stack: Vec<(PathBuf, u32)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            visited += 1;
            if visited > lim.visited || out.len() >= lim.hits {
                return out;
            }
            // The cancel flag is read in batches rather than per entry:
            // a superseded query is answered within a few hundred
            // `readdir` results, and an atomic load per file is a cost
            // paid on every file for the benefit of almost none.
            if visited % 512 == 0 && cancel.load(Ordering::Relaxed) {
                return out;
            }
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if refused(name) {
                continue;
            }
            // `file_type` does not follow links, which is what makes the
            // symlink refusal above true: a link is neither a dir nor a
            // file here, so it falls through both arms.
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < lim.depth {
                    stack.push((e.path(), depth + 1));
                }
            } else if ft.is_file() && grade(name, query).is_some() {
                out.push(e.path());
            }
        }
    }
    out
}

/// A walk in flight on a thread of its own.
///
/// The widget holds one of these between the frame that started it and
/// the frame that collects it. Dropping it cancels the walk and throws
/// the answer away, which is exactly what a superseded query wants — so
/// replacing the field's contents is `self.scan = Some(Scan::start(..))`
/// and nothing else.
pub struct Scan {
    rx: Receiver<Vec<PathBuf>>,
    cancel: Arc<AtomicBool>,
    /// The query this walk was started for, so a late answer can be
    /// checked against the query on screen rather than trusted.
    query: String,
    /// The channel has already answered (or died); nothing more will
    /// come, and `take` stops asking.
    done: bool,
}

impl Scan {
    /// Starts the walk. The thread is detached: it owns nothing but a
    /// sender and its own cancel flag, and it ends by itself when either
    /// the walk finishes or the receiver goes away.
    pub fn start(root: PathBuf, query: String, lim: Limits) -> Scan {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let q = query.clone();
        std::thread::spawn(move || {
            let found = walk(&root, &q, lim, &flag);
            // A send that fails means the widget moved on; there is
            // nobody to tell, and nothing to clean up.
            let _ = tx.send(found);
        });
        Scan { rx, cancel, query, done: false }
    }

    /// The query this walk answers.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The answer, once, without waiting for it. `None` while the walk
    /// is still running — and for ever after it has been taken.
    pub fn take(&mut self) -> Option<Vec<PathBuf>> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(v) => {
                self.done = true;
                Some(v)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                // The thread went away without sending: nothing to
                // report, and nothing to wait for either.
                self.done = true;
                None
            }
        }
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree under a directory of this test's own, torn down at the end.
    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Tree {
            let root = std::env::temp_dir()
                .join(format!("nacelle-search-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tree(root)
        }

        fn file(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            p
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(found: &[PathBuf]) -> Vec<String> {
        let mut v: Vec<String> =
            found.iter().map(|p| crate::rank::file_name(p).to_string()).collect();
        v.sort();
        v
    }

    fn off() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn hidden_directories_and_dot_cache_are_never_walked() {
        let t = Tree::new("hidden");
        t.file("notes.txt");
        t.file(".cache/notes.txt");
        t.file(".config/deep/notes.txt");
        t.file(".notes.txt");
        t.file("work/notes.txt");
        let found = walk(&t.0, "notes", Limits::default(), &off());
        assert_eq!(names(&found), ["notes.txt", "notes.txt"], "the two visible ones");
        assert!(found.iter().all(|p| !p.to_string_lossy().contains("/.")));
        // The rule, stated by itself: today `.cache` is refused BECAUSE
        // it is hidden, and this is what would catch it stopping to be.
        assert!(refused(".cache") && refused(".git") && refused(".notes.txt"));
        assert!(!refused("notes.txt") && !refused("work"));
    }

    #[test]
    fn the_depth_cap_stops_the_walk_wherever_the_tree_keeps_going() {
        let t = Tree::new("depth");
        t.file("a/hit.txt");
        t.file("a/b/hit.txt");
        t.file("a/b/c/hit.txt");
        let lim = |depth| Limits { depth, ..Limits::default() };
        // depth 0 is the root's own entries and nothing below it.
        assert_eq!(walk(&t.0, "hit", lim(0), &off()).len(), 0);
        assert_eq!(walk(&t.0, "hit", lim(1), &off()).len(), 1);
        assert_eq!(walk(&t.0, "hit", lim(2), &off()).len(), 2);
        assert_eq!(walk(&t.0, "hit", lim(3), &off()).len(), 3);
        // And no deeper tree makes it go further than it was allowed.
        assert_eq!(walk(&t.0, "hit", lim(99), &off()).len(), 3);
    }

    #[test]
    fn the_answer_and_the_work_are_both_capped() {
        let t = Tree::new("caps");
        for i in 0..40 {
            t.file(&format!("hit{i}.txt"));
        }
        let lim = Limits { depth: 8, hits: 5, visited: 20_000 };
        assert_eq!(walk(&t.0, "hit", lim, &off()).len(), 5, "answers are capped");
        // The visited cap bounds the WORK, so a query nothing answers
        // still ends: fifteen entries looked at, no answers kept.
        let lim = Limits { depth: 8, hits: 200, visited: 15 };
        assert!(walk(&t.0, "nothing-answers-this", lim, &off()).is_empty());
        let lim = Limits { depth: 8, hits: 200, visited: 10 };
        assert!(walk(&t.0, "hit", lim, &off()).len() < 40, "it gave up on the rest");
    }

    #[test]
    fn an_empty_query_walks_nothing_at_all() {
        let t = Tree::new("empty");
        t.file("a/b/c/thing.txt");
        assert!(walk(&t.0, "", Limits::default(), &off()).is_empty());
        // And a root that is not there is an empty answer, not a panic.
        assert!(walk(Path::new("/nonexistent-nacelle-root"), "x", Limits::default(), &off())
            .is_empty());
    }

    #[test]
    fn a_cancelled_walk_answers_with_what_it_had() {
        let t = Tree::new("cancel");
        t.file("hit.txt");
        let cancel = AtomicBool::new(true);
        // Cancelled before it starts: the batch check is at the 512th
        // entry, so a small tree finishes anyway — what matters is that
        // a cancelled walk never panics and never blocks.
        let found = walk(&t.0, "hit", Limits::default(), &cancel);
        assert!(found.len() <= 1);
    }

    #[test]
    fn a_scan_answers_once_through_the_channel() {
        let t = Tree::new("scan");
        t.file("report.txt");
        let mut scan = Scan::start(t.0.clone(), "report".to_string(), Limits::default());
        assert_eq!(scan.query(), "report");
        // The walk is on another thread, so the answer arrives when it
        // arrives; the widget asks once a frame and this test asks in a
        // loop with the same non-blocking call.
        let mut found = None;
        for _ in 0..2000 {
            if let Some(v) = scan.take() {
                found = Some(v);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let found = found.expect("the walk answered");
        assert_eq!(names(&found), ["report.txt"]);
        assert!(scan.take().is_none(), "an answer is taken once");
    }
}
