// label: CPU
// ref_h: 15.5
// min_h: 9.0
// slot: left
// order: 3
// weight: 26
// u2 §2.5: the title and model move to the host's band; each core is an
// instrument row (label + thin track + value — gauge.style, the theme's
// choice); a rule separates the grid from the footer, where the load
// triplet is a runs line of tabular figures and the temperature — when
// the machine has a sensor — pins itself to the line's right end with
// the script's judgement of the heat (a severity, never a colour).
fn draw() {
    // Each run says WHAT it is — a label, then three readings — and the
    // theme's kind_label_role and kind_reading_role say how big that is.
    // The runs used to name `caption` and `data`, two type roles picked
    // here, which set these numbers at 1.87u against the 3.25u the same
    // kind of number reads at in NETWORK, MEMORY and UPTIME.
    //
    // No runs of spaces between them either: script.runs_gap is the gap,
    // and a literal "  " spent it twice — once as the theme's gap and
    // once as two spaces of whatever width the face gives them.
    let footer = [
        #{ t: "LOAD", role: "label" },
        #{ t: round(host.load1, 2), role: "reading" },
        #{ t: round(host.load5, 2), role: "reading" },
        #{ t: round(host.load15, 2), role: "reading" },
    ];
    // A machine with no temperature sensor says nothing about heat.
    if host.temp != () {
        let heat = if host.temp >= 90.0 {
            "critical"
        } else if host.temp >= 75.0 {
            "warning"
        } else {
            ""
        };
        footer.push(#{
            t: round(host.temp, 0) + "°C",
            role: "reading",
            severity: heat,
            align: "right",
        });
    }
    [
        title("CPU", upper(host.cpu_name)),
        gauges(host.cpu_each, #{ label: "C" }),
        rule(),
        runs(footer, "left"),
    ]
}
