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
        #{ zebra: true, severity_col: 4 },
    )]
}
