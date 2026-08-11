//! WHICH applications the launcher grid is showing — the one fact two
//! widgets in this tree have to agree on.
//!
//! The categories panel (`nacelle-widget-appcats`) is a list and nothing
//! else: clicking a group does not open it there, it POINTS THE GRID at
//! it. So the choice has to travel from one widget to another, and the
//! host has no way to carry it.
//!
//! # Why a process-wide cell, and why that is honest here
//!
//! There is no channel. `PluginApi` carries `draw`, `click`, `wheel`,
//! `chrome`, `sizing` and `drag`, and `ActionC` — the one thing a widget
//! may hand back to the host — has no code that means "another widget
//! should now show something else". A widget cannot name another widget,
//! cannot be told that another widget exists, and cannot be woken by
//! one. That gap is real and it is the TOOLKIT's, not this crate's.
//!
//! Closing it properly means an ABI: a small publish/subscribe surface
//! on `HostApi`, so that a widget states a fact under a name and the
//! host hands it to whoever asked for that name — including across
//! processes and across `dlopen`ed plugins that share no memory at all.
//! That belongs in `libnacelle`, and nothing in this directory may write
//! it.
//!
//! What CAN be done from here is what this module does. Both widgets are
//! BUILT-IN plugins: `nacelle-desktop` links them statically, into one
//! binary, and `appcats` names `appgrid` as a path dependency, so there
//! is exactly one copy of this crate — and therefore exactly one copy of
//! the cell below — in the process. Two widgets reaching the same static
//! is then not a coincidence to be defended against but the linkage the
//! build already guarantees.
//!
//! # What this deliberately does NOT survive
//!
//! A plugin loaded with `dlopen` links its own copy of this crate. Its
//! `SELECTED` is a DIFFERENT cell, so a `.so` build of `appcats` would
//! steer a grid that does not exist and the built-in grid would never
//! hear it. That is not a bug to be fixed inside this file — it cannot
//! be — it is the exact shape of the missing ABI, and the reason the
//! host-side channel is the destination rather than an improvement.
//!
//! So: this is a deliberate stopgap for as long as both widgets are
//! built in, written down here so it reads as a decision and not as an
//! oversight. When `libnacelle` grows the channel, `set` and `get`
//! become its two call sites and nothing else in either widget moves.
//!
//! # Thread safety
//!
//! An `RwLock` rather than a `static mut`: `draw` and `click` are the
//! host's to schedule, this crate is not told on which thread either
//! arrives, and a torn read of a `String` is undefined behaviour rather
//! than a wrong picture. A poisoned lock is stepped over — a panic
//! somewhere else must not turn the launcher into a blank panel.

use std::sync::RwLock;

/// What the grid is pointed at.
///
/// [`Selection::Named`] holds a category NAME rather than an index,
/// for the reason the old drill-down held one too: a rescan rebuilds the
/// grouping, and installing a game where there were none moves every
/// group after it by one. A name still means the same group afterwards;
/// an index quietly becomes a different one.
///
/// An owned `String` rather than the `&'static str` every name in
/// [`crate::cats`] happens to be, because the day a category comes from
/// somewhere other than that fixed table — a user's own group, a menu
/// file — is the day a `'static` bound would have to be unpicked from
/// the ABI this is standing in for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Every installed application, whatever it is grouped under.
    All,
    /// One group, by the name [`crate::cats::group`] gave it.
    Named(String),
}

impl Selection {
    /// Whether this is the whole menu — the question the grid asks to
    /// decide between the alphabetical index and a flat page of tiles.
    pub fn is_all(&self) -> bool {
        matches!(self, Selection::All)
    }

    /// The chosen group's name, or None for the whole menu.
    pub fn name(&self) -> Option<&str> {
        match self {
            Selection::All => None,
            Selection::Named(n) => Some(n),
        }
    }
}

/// The cell itself. `All` at rest, which is what makes ALL APPLICATIONS
/// the selected row of a launcher nobody has clicked yet — the default
/// lives HERE rather than in either widget's constructor, so that a
/// panel created later does not reset a choice already made.
static SELECTED: RwLock<Selection> = RwLock::new(Selection::All);

/// What the grid is showing. Cloned rather than borrowed: holding a
/// read guard across a frame would let a click on one panel block the
/// drawing of another, and a category name is a handful of bytes.
pub fn get() -> Selection {
    SELECTED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Point the grid somewhere. Called by the categories list, and by
/// nothing else in this tree.
pub fn set(what: Selection) {
    *SELECTED.write().unwrap_or_else(|e| e.into_inner()) = what;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, not four: the cell is process-wide, so two tests
    /// writing it would race each other under the default harness.
    #[test]
    fn the_grid_starts_on_the_whole_menu_and_follows_what_it_is_told() {
        // Nobody has clicked: the whole menu, which is what the list
        // draws as the selected row.
        assert_eq!(get(), Selection::All);
        assert!(get().is_all());
        assert_eq!(get().name(), None);

        set(Selection::Named("Utility".to_string()));
        assert_eq!(get(), Selection::Named("Utility".to_string()));
        assert!(!get().is_all());
        assert_eq!(get().name(), Some("Utility"));

        // Switching between two groups, and back to the whole menu —
        // the three transitions the list can produce.
        set(Selection::Named("Game".to_string()));
        assert_eq!(get().name(), Some("Game"));
        set(Selection::All);
        assert!(get().is_all());

        // Put back, so the order tests run in cannot matter.
        set(Selection::All);
    }
}
