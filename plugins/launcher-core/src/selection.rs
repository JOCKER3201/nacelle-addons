//! WHICH applications the launcher grid is showing — the one fact two
//! widgets in this tree have to agree on.
//!
//! The categories panel (`nacelle-widget-appcats`) is a list and nothing
//! else: clicking a group does not open it there, it POINTS THE GRID at
//! it. So the choice has to travel from one widget to another, and it
//! travels through the HOST — `nacelle::channel`, a board of named
//! values the host holds, reached across the plugin boundary by an
//! `api_size`-gated pair of ABI entries.
//!
//! # Why the host carries it
//!
//! Because nothing else can. The loader opens a plugin `RTLD_LOCAL`, on
//! purpose, so `appgrid.so` and `appcats.so` each carry their own copy
//! of this crate — and therefore their own copy of any `static` written
//! here. A process-wide cell in this file agrees with itself and with
//! nobody else the moment the two widgets become two files, which is
//! exactly what they now are. The value has to live where there is only
//! one of it, and that is the host's copy of the toolkit.
//!
//! What travels is BYTES under a name, so neither widget needs a type
//! the other was compiled against and neither has to be rebuilt when the
//! other changes. The value is RETAINED, so a panel created after the
//! click still reads the choice and load order stops mattering. And
//! publishing returns as soon as the value is written, so a click in one
//! panel cannot stall the other one's frame.
//!
//! # The encoding
//!
//! The payload is the chosen group's NAME as UTF-8, and no bytes at all
//! for the whole menu. Two consequences worth stating:
//!
//! * An empty payload and an ABSENT one both read as
//!   [`Selection::All`] — "nobody has chosen yet" and "somebody chose
//!   the whole menu" are the same picture, so they need not be told
//!   apart, and a launcher on a host too old for the channel therefore
//!   shows the whole menu rather than nothing.
//! * A name is never empty ([`crate::cats::group`] returns only groups
//!   that hold something), so no real group can be mistaken for ALL.
//!
//! A NAME rather than an index, for the reason the drill-down held one
//! too: a rescan rebuilds the grouping, and installing a game where
//! there were none moves every group after it by one. A name still means
//! the same group afterwards; an index quietly becomes a different one.

use nacelle::channel;

/// The topic both launcher widgets meet under. A constant in one file
/// because a topic is an agreement: two spellings of it are two boards.
pub const TOPIC: &str = "launcher.category";

/// What the grid is pointed at.
///
/// An owned `String` rather than the `&'static str` every name in
/// [`crate::cats`] happens to be, because a name that arrived over the
/// channel is bytes this process owns — and because the day a category
/// comes from somewhere other than that fixed table (a user's own group,
/// a menu file) a `'static` bound would have to be unpicked again.
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

    /// What goes on the board.
    fn bytes(&self) -> &[u8] {
        self.name().unwrap_or("").as_bytes()
    }

    /// What came off it. Bytes that are not UTF-8 read as the whole menu
    /// rather than as a lossy group name nothing is filed under: a
    /// publisher this build does not understand must not be able to
    /// invent a category.
    fn from_bytes(data: &[u8]) -> Selection {
        match std::str::from_utf8(data) {
            Ok(name) if !name.is_empty() => Selection::Named(name.to_string()),
            _ => Selection::All,
        }
    }
}

/// One widget's view of the choice, and the only thing either launcher
/// widget holds.
///
/// It exists because a reader wants two different things from the board
/// and they cost different amounts: "has this changed" is a sequence
/// number, and "what is it now" is a copy. [`Watch::poll`] asks the cheap
/// question every frame and the expensive one only in the frames where
/// the answer moved — which is what the channel's sequence number is
/// for, and what keeps a grid from rebuilding its page sixty times a
/// second to arrive at the page it already had.
pub struct Watch {
    /// The sequence number of the value in `sel`. 0 means nothing has
    /// ever been published — the state a launcher nobody has clicked is
    /// in, and the state it stays in on a host with no channel.
    seq: u64,
    sel: Selection,
}

impl Watch {
    /// A view that has not looked yet.
    ///
    /// It reads NOTHING here, and that is the point: constructing a
    /// widget is not a frame, it happens whenever the host felt like
    /// making one, and a view whose answer depended on the moment it was
    /// built would be a widget that behaves differently for having been
    /// created a millisecond earlier. The first [`Watch::poll`] — which
    /// every widget does at the top of every frame — adopts whatever is
    /// standing, so a panel opened after a click still shows the choice
    /// already made, one frame after it exists and before it draws.
    pub fn new() -> Watch {
        Watch { seq: 0, sel: Selection::All }
    }

    /// What the grid is showing, as of the last [`Watch::poll`].
    pub fn get(&self) -> &Selection {
        &self.sel
    }

    /// Reads the board, and answers whether the choice MOVED.
    ///
    /// The sequence number is compared rather than the value: two
    /// publications of the same name are still two publications, and a
    /// reader that skipped work on the second would be right — but only
    /// by accident, and the cheap comparison is the one that is always
    /// right.
    pub fn poll(&mut self) -> bool {
        if channel::seq(TOPIC) == self.seq {
            return false;
        }
        match channel::read(TOPIC) {
            Some(m) => {
                self.seq = m.seq;
                let sel = Selection::from_bytes(&m.data);
                let moved = sel != self.sel;
                self.sel = sel;
                moved
            }
            // Between the two calls the topic cannot go away — nothing
            // in this toolkit unpublishes — so this is the host that has
            // no channel at all, answering 0 to both. Nothing changed.
            None => false,
        }
    }

    /// Points the grid somewhere, and adopts the choice at once.
    ///
    /// Adopted locally as well as published, for two reasons: the rest
    /// of THIS frame's hit list and the next draw agree without waiting
    /// for a read back, and on a host too old for the channel the row a
    /// click chose still marks itself — the panel is honest about what
    /// it was told even when nothing can hear it. A refused publish
    /// leaves `seq` where it was, so the first value that IS published
    /// still arrives.
    pub fn set(&mut self, what: Selection) {
        let seq = channel::publish(TOPIC, what.bytes());
        if seq != 0 {
            self.seq = seq;
        }
        self.sel = what;
    }
}

impl Default for Watch {
    fn default() -> Self {
        Watch::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, not six: the board is process-wide, so two tests
    /// writing it would race each other under the default harness — the
    /// reason the cell this replaces had one test too.
    ///
    /// What it proves is the thing the cell could not do: TWO WATCHES
    /// THAT SHARE NO MEMORY agree. Each holds its own sequence number
    /// and its own copy of the value, exactly as two widgets in two
    /// `.so` files each hold their own copy of this crate; the only
    /// thing between them is the host's board.
    #[test]
    fn one_panels_choice_reaches_another_that_shares_no_memory_with_it() {
        // Nobody has clicked: the whole menu, which is what the list
        // draws as the selected row and what the grid draws its
        // alphabetical index for.
        let mut cats = Watch::new();
        let mut grid = Watch::new();
        assert_eq!(*grid.get(), Selection::All);
        assert!(grid.get().is_all());
        assert_eq!(grid.get().name(), None);
        assert!(!grid.poll(), "an unpublished board never moves");

        // The list points the grid at a group. The grid is told on its
        // next frame, and told ONCE.
        cats.set(Selection::Named("Utility".to_string()));
        assert_eq!(cats.get().name(), Some("Utility"), "and adopts it at once");
        assert!(grid.poll(), "the grid hears it");
        assert_eq!(*grid.get(), Selection::Named("Utility".to_string()));
        assert!(!grid.get().is_all());
        assert!(!grid.poll(), "and does not rebuild its page again for it");

        // Switching moves the choice rather than adding a second one.
        cats.set(Selection::Named("Game".to_string()));
        assert!(grid.poll());
        assert_eq!(grid.get().name(), Some("Game"));

        // And the top row takes it back.
        cats.set(Selection::All);
        assert!(grid.poll());
        assert!(grid.get().is_all());

        // A panel created after every one of those clicks reads the
        // choice standing on its FIRST frame — that is what a RETAINED
        // value means, and it is why load order stops mattering. Made
        // and not yet drawn, it is on the whole menu, which is the
        // launcher's own default and never a wrong group.
        cats.set(Selection::Named("Office".to_string()));
        let mut late = Watch::new();
        assert!(late.get().is_all(), "a view that has not looked has seen nothing");
        assert!(late.poll(), "and its first frame catches up");
        assert_eq!(late.get().name(), Some("Office"));

        // The wire format, from both ends. It is bytes on a board, so it
        // is worth stating what those bytes are rather than trusting the
        // round trip alone.
        assert_eq!(Selection::All.bytes(), b"");
        assert_eq!(Selection::Named("Utility".into()).bytes(), b"Utility");
        assert_eq!(Selection::from_bytes(b""), Selection::All);
        assert_eq!(Selection::from_bytes(b"Utility"), Selection::Named("Utility".into()));
        // Bytes this build cannot read are the whole menu, never an
        // invented group: a name nothing is filed under would be an
        // empty grid with no way to say why.
        assert_eq!(Selection::from_bytes(&[0xff, 0xfe]), Selection::All);

        // Put the board back, so the order tests run in cannot matter.
        cats.set(Selection::All);
    }
}
