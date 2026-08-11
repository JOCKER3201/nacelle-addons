# nacelle-widgets

The widgets of the [nacelle-desktop](https://github.com/JOCKER3201/nacelle-desktop)
project.

A widget is a directory. Most are **Rhai scripts** in `widgets/` — the
ordinary way to write one, sandboxed by construction and the same file
on every platform. The widgets directory is split by the kind of board
a widget may be placed on, and the category comes from the directory
alone:

| directory | board |
|---|---|
| `widgets/board/` | the ordinary boards — home and its horizontal arms |
| `widgets/appgrid/` | APPGRID, the bottom fixture board |
| `widgets/search_and_ai/` | SEARCH AND AI, the top fixture board |

The four **core widgets** — the control panel, the terminal, the file
browser and the keyboard — are Rust crates in `plugins/`. They are not
installed from here: nacelle-desktop links them into its own binary, so
the program works with nothing installed at all and no file can take
them away. Their code lives in this repository, and the desktop pulls
the crates as git dependencies. Built with their default `dyn` feature
they still produce the classic dlopen `.so`, which is how a
third-party compiled widget would ship.

Both kinds implement the same `Widget` contract from
[libnacelle](https://github.com/JOCKER3201/libnacelle), so the
application drives them identically and knows neither by name. The
directory name is the widget's name, and that is what ties it to its
place in a layout.

> **THIS PROJECT WAS WRITTEN ENTIRELY BY ANTHROPIC'S CLAUDE AI MODELS.**

> **Compiled `.so` widgets are native code with the program's full
> privileges and no sandbox around them. Install one only from a source
> you trust — the same care an AUR package or any other user repository
> deserves. Rhai scripts carry no such risk: they are sandboxed by
> construction and can only read the values the host exposes.**

## Installation

```sh
make install        # install scripts to ~/.local/share/nacelle-desktop
sudo make install   # install scripts to /usr/local/share/nacelle-desktop
```

The program searches the user's directory first, then the system ones,
and the first copy of a given name wins — so a widget you install for
yourself shadows a packaged one without touching root's files. A script
already installed is never overwritten: a widget you have edited
survives an upgrade untouched. `make uninstall` removes only the `.so`
plugin leftovers of older releases; the widget directories, their
scripts and their assets are the user's and stay.

## Writing your own

A widget is a directory whose name is the widget's name, holding one
file: `<name>.rhai` or `<name>.so`. Drop it under the category
directory for the board it belongs on — `board/`, `appgrid/` or
`search_and_ai/` in the installed widgets directory — and the program
offers it in the layout editor of that kind of board. No rebuild, no
registration list, nothing to edit anywhere else. (A widget directory
placed at the top level, the pre-split arrangement, still works and
counts as a board widget.)

There is no library to depend on. What the application and a widget must
agree on before either has drawn — where a widget's clickable controls
are, so the pointer can become a hand — lives in
[libnacelle](https://github.com/JOCKER3201/libnacelle) as
`nacelle::geometry`, which both sides already depend on.

## License

MIT — see [LICENSE](LICENSE).
