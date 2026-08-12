//! The launcher's shared half — everything the two launcher widgets
//! are built out of, and not one line that draws a widget.
//!
//! APPLICATIONS (`nacelle-widget-appgrid`) and CATEGORIES
//! (`nacelle-widget-appcats`) are two views of one launcher: the same
//! menu, found by the same scan, grouped by the same reading of the
//! menu specification, drawn with the same chamfer and the same
//! scrollbar. Two copies of any of that would be two launchers that
//! drift apart on the first token or the first category that moves.
//!
//! # Why it is a crate and not a module of the grid
//!
//! It used to be five public modules of `nacelle-widget-appgrid`, with
//! the categories widget naming that crate as a path dependency. That
//! made the categories widget unshippable: a plugin is a file, a file
//! exports exactly one `nacelle_plugin_attach`, and linking a crate
//! that exports one into another that exports one is a library with
//! two entry points — a loader picking either would be picking at
//! random. The shared half moving into a crate with NO entry point is
//! the structural answer, so that both widgets are ordinary addons and
//! neither is the other's host.
//!
//! Nothing here is a widget. There is no `WIDGET` constant, no attach
//! symbol and no `dyn` feature, because there is nothing here for a
//! host to load: a library the host never sees, only the two addons
//! that link it.
//!
//! # What each module is
//!
//! * [`desktop`] — the system's ONE XDG scan: where desktop entries
//!   live, what a `[Desktop Entry]` says, and how a click becomes a
//!   running application.
//! * [`cats`] — the system's ONE reading of the Desktop Menu
//!   Specification's categories: which groups a menu actually has and
//!   what is in each.
//! * [`tile`] — the system's ONE tile grid: the theme it reads, the
//!   arithmetic from a content box to rows of tiles, and the shapes a
//!   tile is made of.
//! * [`sections`] — the alphabetical index over those tiles.
//! * [`selection`] — the ONE fact the two widgets have to agree on, and
//!   the host channel that carries it between them. Read its head
//!   before either widget: the choice does not live in this crate at
//!   all, because two `.so` files carry two copies of this crate and
//!   only the host has one of anything.
//!
//! Nothing here decides a colour, a length or a word: every one arrives
//! from the theme through ABI 5/6 tokens, and a missing token degrades
//! through the raw answers the ABI itself gives, never through a number
//! that used to be the design.

pub mod cats;
pub mod desktop;
pub mod sections;
pub mod selection;
pub mod tile;
