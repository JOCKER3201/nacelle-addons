# nacelle-addons

The addons of the [nacelle-desktop](https://github.com/JOCKER3201/nacelle-desktop)
project.

An addon is one file, and its stem is its name. Most are **Rhai
scripts** in `scripts/` — the ordinary way to write one, sandboxed by
construction and the same file on every platform.

## The addon describes itself

The program keeps no list of widgets. What exists is what it finds in
the addons directory, so everything it must know about an addon *before*
that addon draws — the label the layout editor shows, the heights the
layout engine solves for, the kind of board it may be placed on, where
in the arrangement it wants to stand — is declared by the addon itself.

A **script** declares it in header pragmas within its first lines,
read as text and never by running the script:

```rhai
// label: SYSTEM INFO
// ref_h: 4.5
// min_h: 4.5
// slot: left
// order: 1
```

A **compiled plugin** declares it in a `<name>.meta` file installed
beside `<name>.so`, one `key = value` per line and `#` for a comment:

```ini
label = SHELL
ref_h = 60.0
min_h = 12.0
slot = center
anchor = top
```

It is a separate file for the reason KDE's plasmoids and GNOME's
extensions keep one: the host has to know what a widget *is* before it
decides to load anybody's code.

| key | meaning | default |
|---|---|---|
| `label` | the name the layout editor shows | the file's stem in capitals |
| `ref_h` | height in vh at which the widget draws at 100% scale | `10.0` |
| `min_h` | height in vh below which its last content row would leave the screen | `6.0` |
| `category` | which kind of board it may be placed on | `board` |
| `slot` | `left`, `center` or `right` — which column of a generated arrangement it asks for | whichever side has room |
| `order` | where in that column, lowest first | registry order |
| `weight` | how much of a shared column it wants | as much of it as the widget is tall |
| `anchor` | `top`, `bottom` or `bar` — a pinned edge instead of flowing with the column | it flows |
| `essential` | `true` — switching it off would leave the user no way back, so the editor never offers to remove it | removable |

| category | board |
|---|---|
| `board` (or nothing) | the ordinary boards — home and its horizontal arms |
| `appgrid` | APPGRID, the bottom fixture board |
| `search_and_ai` | SEARCH AND AI, the top fixture board |

Every key is optional, and an unknown one is ignored: an addon that
declares nothing is still an addon, and an addon written for a later
version of the program still loads in this one. The name is never a
key — it is the file's stem, so no addon can claim to be another one
by editing a comment.

The **core widgets** — the control panel, the terminal, the file
browser, the keyboard, the application launcher, its category list and
the AI panel — are Rust crates in `plugins/`, and they are **installed exactly like
everything else**: `make` builds each into its own `.so`, `make
install` puts it in `addons/plugins/<name>.so` with its `<name>.meta`
beside it, and the program finds it by the same directory scan that
finds anybody's addon. The core is not a privileged kind of widget. It
is the widgets this repository happens to ship.

Building each of them with `--no-default-features` drops the dlopen
attach symbol and leaves the crate linkable, which is how a host that
wants a core no file can uninstall pulls them in as dependencies
instead. The two builds are the same code; only the entry point
differs, and a linked name shadows an installed file of the same name
because what the layout editor offers has to be what will draw.

One crate in `plugins/` is not a widget. `launcher-core` is the
launcher's shared half — the XDG scan, the categories, the tile grid,
the alphabetical index and the cell the two launcher widgets meet in —
which both of them link and neither owns. It exports no attach symbol
and installs nothing; it exists so that a single `.so` never carries
two entry points, which is what a loader cannot choose between.

Both kinds implement the same `Widget` contract from
[libnacelle](https://github.com/JOCKER3201/libnacelle), so the
application drives them identically and knows neither by name. The
file's stem is the addon's name, and that is what ties it to its place
in a layout.

> **THIS PROJECT WAS WRITTEN ENTIRELY BY ANTHROPIC'S CLAUDE AI MODELS.**

> **Compiled `.so` addons are native code with the program's full
> privileges and no sandbox around them. Install one only from a source
> you trust — the same care an AUR package or any other user repository
> deserves. Rhai scripts carry no such risk: they are sandboxed by
> construction and can only read the values the host exposes.**

## Installation

```sh
make                # build the compiled addons (cargo, release)
make install        # build, then install to ~/.local/share/nacelle-desktop/addons
sudo make install   # build, then install to /usr/local/share/nacelle-desktop/addons
```

`make install` builds what it installs, so one command per repository
is still the whole story. A packager who has already built installs
without a rebuild with `make install-scripts install-plugins`, and a
compiled addon that was not built is named in the output rather than
quietly left out — an addon directory missing it is a program missing
that widget.

The program searches the user's directory first, then the system ones,
and the first copy of a given name wins — so an addon you install for
yourself shadows a packaged one without touching root's files. A script
already installed is never overwritten: an addon you have edited
survives an upgrade untouched. A `.so` *is* overwritten, because nobody
edits a shared object — it is a build artefact of this repository, and
a stale one against a newer toolkit is a plugin the loader turns away.
Installs from before the addons layout
(`widgets/<category>/<name>/`) are migrated automatically — by the
program at startup for your own directory, and by `make install` for a
tree the program cannot write; either way every script is rescued into
the new place first, never overwriting.

## Writing your own

An addon is one file: `addons/scripts/<name>.rhai` or
`addons/plugins/<name>.so` in the installed data directory. Drop it
there — with a `// category:` pragma if it belongs on a fixture board,
and a `// slot:` pragma if it wants a particular column of the
arrangement the program falls back to — and it is offered in the layout
editor of that kind of board. No rebuild, no registration list, nothing
to edit anywhere else: there is no table of widgets in the program at
all, so a machine with no addons installed has no widgets, exactly as a
machine with no theme installed draws like a page with no stylesheet.

There is no library to depend on. Where an addon's clickable controls
are is the addon's own business: the application asks it (the widget
interface's `pointer` entry) before it turns the cursor into a hand,
rather than keeping a copy of anybody's geometry.

## License

MIT — see [LICENSE](LICENSE).
