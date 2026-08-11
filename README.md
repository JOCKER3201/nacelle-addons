# nacelle-addons

The addons of the [nacelle-desktop](https://github.com/JOCKER3201/nacelle-desktop)
project.

An addon is one file, and its stem is its name. Most are **Rhai
scripts** in `scripts/` — the ordinary way to write one, sandboxed by
construction and the same file on every platform. The kind of board an
addon may be placed on is declared in the addon itself, in a header
pragma within its first lines:

```rhai
// category: appgrid
```

| category | board |
|---|---|
| `board` (or nothing) | the ordinary boards — home and its horizontal arms |
| `appgrid` | APPGRID, the bottom fixture board |
| `search_and_ai` | SEARCH AND AI, the top fixture board |

The four **core widgets** — the control panel, the terminal, the file
browser and the keyboard — are Rust crates in `plugins/`. They are not
installed from here: nacelle-desktop links them into its own binary, so
the program works with nothing installed at all and no file can take
them away. Their code lives in this repository, and the desktop pulls
the crates as git dependencies. Built with their default `dyn` feature
they still produce the classic dlopen `.so`, which is how a
third-party compiled addon would ship — installed as
`addons/plugins/<name>.so`.

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
make install        # install scripts to ~/.local/share/nacelle-desktop/addons
sudo make install   # install scripts to /usr/local/share/nacelle-desktop/addons
```

The program searches the user's directory first, then the system ones,
and the first copy of a given name wins — so an addon you install for
yourself shadows a packaged one without touching root's files. A script
already installed is never overwritten: an addon you have edited
survives an upgrade untouched. Installs from before the addons layout
(`widgets/<category>/<name>/`) are migrated automatically — by the
program at startup for your own directory, and by `make install` for a
tree the program cannot write; either way every script is rescued into
the new place first, never overwriting.

## Writing your own

An addon is one file: `addons/scripts/<name>.rhai` or
`addons/plugins/<name>.so` in the installed data directory. Drop it
there — with a `// category:` pragma if it belongs on a fixture board —
and the program offers it in the layout editor of that kind of board.
No rebuild, no registration list, nothing to edit anywhere else.

There is no library to depend on. What the application and an addon must
agree on before either has drawn — where an addon's clickable controls
are, so the pointer can become a hand — lives in
[libnacelle](https://github.com/JOCKER3201/libnacelle) as
`nacelle::geometry`, which both sides already depend on.

## License

MIT — see [LICENSE](LICENSE).
