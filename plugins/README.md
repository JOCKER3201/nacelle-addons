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

## Reading your own settings

A plugin never opens a settings file. It NAMES the addon whose settings
it wants and the host answers the text, which is the same boundary the
clipboard has: hand a plugin a path and the plugin picks the file; hand
it a name out of a namespace the host controls and the worst a wrong
name can do is miss.

```rust
#[derive(serde::Deserialize, Default)]
#[serde(default)]                    // REQUIRED — see below
struct Config { rows: u32, format: String }

// in create():
let (cfg, _origin) = nacelle::settings::load::<Config>("clock", "");

// once a frame, at the top of draw — a u32 compare, never a parse:
let e = nacelle::settings::epoch();
if e != self.cfg_epoch {
    self.cfg_epoch = e;
    self.cfg = nacelle::settings::load::<Config>("clock", "").0;
}
```

The empty second argument is an addon with ONE settings file
(`addons/clock.ron`); a member name is one of a directory
(`addons/search/engines.ron`), which an addon gets when it needs a
second file and not before.

`#[serde(default)]` on the container is not style. RON parses all or
nothing, so without it a document missing one field — an old file, a
file written before the addon grew a setting, a file where the user set
only what they cared about — fails whole and costs the user every
setting in it. Do not add `deny_unknown_fields` either: a key from a
later version must not cost the keys this one understands.

Two things the toolkit does and a plugin must not repeat: it reports a
document that will not parse, once, with the path and the position — a
second copy of that diagnostic is noise — and it answers your own
defaults when there is no file at all, which is not an error and needs
no mention anywhere.

One thing it CANNOT do and a plugin therefore must. `Origin::Refused`,
under a name a plugin spells out itself, means the host installed no
settings directories: nothing was looked for, so there is no path to
report and the toolkit has nothing to name. Your addon does — it knows
which file its user would have written — so say so, once, at create:

```rust
if origin == nacelle::settings::Origin::Refused {
    eprintln!("clock: this host delivers no addon settings \
               — addons/clock.ron, if you wrote one, is not being read");
}
```

Running on the defaults is correct there. Running on them in silence,
on a machine whose owner edited the file, is the failure this whole
arrangement exists to refuse.

`launcher-core` has no settings file for the same reason it has no
attach symbol: it is not an addon. Nobody would ever read the file, so
nobody should ever write one.

> Compiled widgets are rebuilt for every release and separately for each
> platform and architecture, and run with the host's full privileges. If
> a script can do the job, write a script.
