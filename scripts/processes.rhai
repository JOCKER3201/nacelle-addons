fn draw() {
    let body = [];
    for p in host.processes {
        let row = ["" + p.pid, p.name, round(p.cpu, 1) + "%", round(p.mem, 1) + "%"];
        // The script's judgement of the data (u2 §2.7): a runaway
        // process tints its row through the severity set. The word is
        // consumed as style, never drawn as a cell, and an ordinary row
        // carries none.
        if p.cpu >= 90.0 {
            row.push("critical");
        } else if p.cpu >= 75.0 {
            row.push("warning");
        }
        body.push(row);
    }
    [table(
        [
            ["PID", "right"],
            ["NAME", "left"],
            // The CPU number is unchanged; the hairline track behind it
            // is a second reading of the same value (u2 §2.7).
            ["CPU", "right", #{ kind: "bar", of: 100.0 }],
            ["MEM", "right"],
        ],
        body,
        1,
        // Striping helps a dense list scan; how often and in what tint
        // is the theme's (script.table_zebra_every, component.table.zebra).
        //
        // The table answers the pointer (F2 §2.2): a click on a heading
        // sorts by that column and clicking it again turns the order
        // around, a click on a row keeps it — by PID, so the selection
        // survives the second-by-second rewrite of the process table and
        // any sort applied to it — and a name the ellipsis cut short
        // gives itself in full when the pointer rests on it.
        //
        // No opening `sort`: the arrangement the collector hands over is
        // the one this widget has always shown, and the user's first
        // click is what changes it. No `scroll` either — the table shows
        // what fits, exactly as before.
        #{
            zebra: true,
            severity_col: 4,
            id: "procs",
            interactive: true,
            select: "row",
            key: 0,
            tooltip: true,
        },
    )]
}
