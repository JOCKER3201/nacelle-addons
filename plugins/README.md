# Compiled widgets

Widgets that a Rhai script cannot express, each a `cdylib` the host
loads with `dlopen`.

A plugin exports `ng_plugin_attach`, which takes the host's function
table and returns its own. The host refuses a library without that
symbol: attaching is also what stops a plugin from keeping a second copy
of state that is supposed to exist once per program, and that decision
cannot be left to the plugin.

Everything a plugin draws goes through the host's table. It never sees a
`Ctx` — that holds Rust references whose layout is not promised across a
library boundary — only an opaque handle it passes back.

Build one with `cargo build --release`; the result goes into the
widget's directory as `<name>.so`.

> Compiled widgets are rebuilt for every release and separately for each
> platform and architecture, and run with the host's full privileges. If
> a script can do the job, write a script.
