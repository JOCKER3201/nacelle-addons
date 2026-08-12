// label: UPTIME
// ref_h: 8.0
// min_h: 7.0
// slot: left
// order: 5
// weight: 8
// A widget that exists only as a script — there is no compiled code
// behind it. It also shows what a script can do that a fixed list of
// rows cannot: decide what to show.

fn draw() {
    let lines = [
        ["UP",   uptime(host.uptime)],
        ["HOST", upper(host.name)],
        ["USER", upper(host.user)],
    ];
    // Only worth a line when there is something to put in it.
    if host.kernel != "" {
        lines.push(["KERNEL", host.kernel]);
    }
    // Machines without a battery say nothing rather than "--".
    let power = if host.charging == () {
        ""
    } else if host.charging {
        "CHARGING"
    } else {
        "BATTERY"
    };
    [
        // The heading and the power word are the chrome declaration now
        // (u2 §2.3): the HOST's title band draws them — same strings,
        // same data — and the element takes no body height.
        title("UPTIME", power),
        // The two-column key:value grid of u2 §2.3: four pairs become
        // two lines, and the shared label column starts every value at
        // one x instead of the old ragged cloud. With three pairs (no
        // kernel) the odd one spans the full width.
        rows(lines, #{ columns: 2, label_width: "max" }),
    ]
}
