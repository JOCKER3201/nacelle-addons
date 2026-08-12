// label: HARDWARE
// ref_h: 6.5
// min_h: 6.5
// slot: left
// order: 2
// weight: 7
// nacelle-desktop widget. This file IS the widget: called once a frame, it
// returns the elements to draw. Any assets it needs live beside it.
//
// Everything readable is in `host`; everything doable is the element and
// formatting functions. There is no way to reach the filesystem, the
// network or another process, because no such function exists here.
//
// u2 §2.4: the title is a chrome declaration — the host's band draws it —
// and the three rows are compact, with one shared label column so every
// value starts at the same x and long models trim instead of shrinking.
//
// `density` and `label_width` are arrangement — how the lines are packed
// and where the column breaks — and both resolve to numbers the theme
// owns. The type is not here at all: these are rows, and a row is set the
// way script.rows_*_role says. The value_role this call used to carry put
// three machine names in the `data` role — 1.87u, and a monospace face —
// where the same kind of string reads at 3.25u in every other panel.
//
// This panel is also the one that pays if the toolkit gets tabular
// figures wrong: its values are the only PROSE any widget puts in the
// value half, and the value role is tabular. A figure box that swallowed
// word spaces spaced `THINKPAD X1 CARBON GEN 9` out by half a figure per
// gap. The rule is that a mark joins the box where it is part of a
// number, so the digits in a model name still hold a column and the words
// around them are the face's own — nothing for this file to ask for.
fn draw() {
    [
        title("HARDWARE"),
        rows([
            ["MANUFACTURER", upper(host.manufacturer)],
            ["MODEL",        upper(host.model)],
            ["CHASSIS",      host.chassis],
        ], #{ label_width: "max", density: "compact" }),
    ]
}
