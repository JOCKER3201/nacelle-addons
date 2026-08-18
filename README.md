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

## Settings

What an addon *looks like* is the theme's, always. What it *does* on
your machine — which directory it walks, how much of it, whether it
follows the terminal — is yours, and it lives in one file per addon:

```
~/.config/nacelle/addons/<name>.ron      your own
/etc/xdg/nacelle/addons/<name>.ron       the packaged answer
```

The name of the file is the name of the addon. An addon that needs a
*second* settings file gets a directory of that name instead; none of
the ones shipped here does. The first file found is the whole answer —
your file replaces the packaged one rather than merging with it — and
**every key is optional**: one you do not write keeps the default below,
so a file stating a single line is a complete and correct file. A key
this version has never heard of is ignored, so a file written for a
later one still loads.

A file the program cannot use is *reported* — the path and the position
— and the addon runs on its defaults until it is fixed. It is never
ignored in silence, because a widget that quietly looks factory-fresh
gives you no reason to suspect the file you just edited. Two failures
count as one file: a document that is not RON at all, and a document
that is RON and says something the addon has no room for — `hidden:
"yes"` where a yes-or-no goes. The second is the likelier of the two
and the one nothing but the addon can see, so it is reported by the
same rule rather than left to a shrug.

It is reported in two places. On **stderr**, once, which is where it
is of use to anybody starting the program from a terminal; and in the
**settings window**, under `SETTINGS — ADDONS`, which is where it is
of use to everybody else — a desktop session's stderr goes to a
journal nobody has open. That page names the directory your files go
in, lists every one the program could not use, and says so plainly
when there is nothing to report. It only ever *shows*: these files are
written by hand, and no settings window edits them yet. When one does,
it will save through the toolkit, which leaves the previous contents
in `<name>.ron.bak`.

Everything standing in those directories is read once at startup,
rather than only when the addon that owns it is asked to draw. What
the program can judge on its own — whether the file is a document at
all — is therefore judged for every file that exists, including one
belonging to a widget you have not placed on any board. Whether the
document *fits* is the addon's own reading, so that half arrives when
the addon first runs, and joins the same report.

A desktop that delivers no addon settings *at all* — one older than
these files, or one that has not been wired to them — is the worse
failure and gets a line of its own on that page and on stderr, because
it has no bad files to list and is ignoring every file on the machine.

Two of the shipped addons have settings. The others have none, and no
file: an empty settings file is a promise that something can be set.

### `filesystem.ron` — the file browser

| key | meaning | default |
|---|---|---|
| `start` | the directory the panel opens in; `~` is expanded, empty means your home | `""` |
| `follow_shell` | walk with the active terminal tab's working directory | `true` |
| `hidden` | list entries whose name begins with a dot | `true` |
| `directories_first` | gather directories above files; otherwise sort by name alone | `true` |

```ron
// ~/.config/nacelle/addons/filesystem.ron
(
    hidden: false,
    start: "~/work",
)
```

`start` is read when the panel is created and not again: it says where
browsing *begins*, and re-reading it would pull you out of wherever you
had got to. Everything else takes effect at once.

### `search.ron` — the search panel

The applications half needs no settings: it shows what the freedesktop
specification says is installed. Everything here is about the file walk.

| key | meaning | default |
|---|---|---|
| `files` | walk the filesystem at all; `false` leaves you the applications | `true` |
| `root` | where the walk starts; `~` is expanded, empty means your home | `""` |
| `depth` | directories below the root; `0` walks the root itself only | `8` |
| `hits` | how many files one walk keeps before it stops | `200` |
| `visited` | how many entries it may look at before giving up on the rest | `20000` |

```ron
// ~/.config/nacelle/addons/search.ron
(
    root: "~/projects",
    depth: 4,
)
```

The walk always refuses hidden directories and symlinks, whatever the
root — `.git` and `node_modules` are not answers anybody wants, and a
link is not a place.

The Rhai scripts in `scripts/` have no settings files: the script host
has no entry to hand one over with, so a script is edited directly, which
is what a script is for.

> **THIS PROJECT WAS WRITTEN ENTIRELY BY ANTHROPIC'S CLAUDE AI MODELS.**

> **Compiled `.so` addons are native code with the program's full
> privileges and no sandbox around them. Install one only from a source
> you trust — the same care an AUR package or any other user repository
> deserves. Rhai scripts carry no such risk: they are sandboxed by
> construction and can only read the values the host exposes.**

## Installation

```sh
make                # build the compiled addons (cargo, release)
make install        # build, then install to ~/.local/share/nacelle/addons
sudo make install   # build, then install to /usr/local/share/nacelle/addons
```

The directory is named after the nacelle **family**, not after one of
its programs: `nacelle-themes` installs beside it and the program looks
there first. Installations made before 2026-08-18 landed under
`share/nacelle-desktop` instead; nothing of theirs is moved or removed,
because both names are read — the family's first, the old one directly
behind it — and this installer reads both too. `make check-install`
answers the question for your own tree: it installs into a throwaway
directory and says where things went.

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
"Already installed" is asked of both directory names, so a script you
edited under the old one is not shadowed by a factory copy laid above
it. Installs from before the addons layout
(`widgets/<category>/<name>/`) are migrated automatically — by the
program at startup for the directory it writes to, and by
`make install` under either name; either way every script is rescued
into the new place first, never overwriting, and only an emptied
directory disappears.

`make uninstall` takes back what this repository installed, under both
names: the scripts it shipped that you have not edited, and the
libraries it built. Anything else in there is somebody's, including
every addon in a pre-addons `widgets/` tree, and it is left alone.

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
