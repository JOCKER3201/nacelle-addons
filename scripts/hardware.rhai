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
fn draw() {
    [
        title("HARDWARE"),
        rows([
            ["MANUFACTURER", upper(host.manufacturer)],
            ["MODEL",        upper(host.model)],
            ["CHASSIS",      host.chassis],
        ], #{ value_role: "data", label_width: "max", density: "compact" }),
    ]
}
