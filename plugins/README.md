# Compiled widgets

Widgets that a Rhai script cannot express, each a `cdylib` the host
loads with `dlopen`.

A plugin exports `nacelle_plugin_attach`, which takes the host's
function table and returns its own. The host refuses a library without
that symbol, and a library with **two** of them is a broken file rather
than a rich one: the loader looks the name up once and would get
whichever the linker happened to put first. So a `.so` here is built
from exactly one crate that exports it.

Everything a plugin draws goes through the host's table. It never sees a
`Ctx` — that holds Rust references whose layout is not promised across a
library boundary — only an opaque handle it passes back.

Build them with `make` at the top of this repository (`cargo build
--release`); `make install` puts each one at `addons/plugins/<name>.so`
with the crate's `<name>.meta` beside it — the label, the reference and
minimum heights and the category, which the host reads INSTEAD of the
library, before it decides to load it. The crate exports the same two
as one `pub const WIDGET`, `include_str!`ing that very file, so a host
that links the crate instead of loading the file cannot describe the
widget differently. A plugin installed without its `.meta` still works:
it gets its name in capitals and the standard heights.

Cargo names a `cdylib` after its crate, so the build output is
`libnacelle_widget_<name>.so`; the addon's name is its FILE's stem, and
the installer renames it to `<name>.so`. Nothing but the installer
knows about that mapping.

## `dyn`, and the crate that has no attach symbol

Every widget crate here has a default `dyn` feature, and it is the
attach symbol. Turned off, the crate is an ordinary Rust library a host
can link — the path `nacelle-desktop` takes for a core no file can
uninstall — and several linked widgets cannot then collide over one
`nacelle_plugin_attach`. The code is the same either way.

`launcher-core` is not a widget and has no such feature. It is the
launcher's shared half — the XDG scan, the categories, the tile grid,
the alphabetical index and the cell APPLICATIONS and CATEGORIES meet in
— which both of them link and neither owns. It exports no attach
symbol, so linking it leaves each of the two with exactly one, which is
what lets both ship as files. Before it existed, CATEGORIES depended on
APPLICATIONS' crate and could not be a file at all.

> Compiled widgets are rebuilt for every release and separately for each
> platform and architecture, and run with the host's full privileges. If
> a script can do the job, write a script.
