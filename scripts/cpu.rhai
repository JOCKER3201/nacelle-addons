// u2 §2.5: the title and model move to the host's band; each core is an
// instrument row (label + thin track + value — gauge.style, the theme's
// choice); a rule separates the grid from the footer, where the load
// triplet is a runs line of tabular figures and the temperature — when
// the machine has a sensor — pins itself to the line's right end with
// the script's judgement of the heat (a severity, never a colour).
fn draw() {
    let footer = [
        #{ t: "LOAD", role: "caption" },
        #{ t: "  ", role: "data" },
        #{ t: round(host.load1, 2), role: "data" },
        #{ t: "  ", role: "data" },
        #{ t: round(host.load5, 2), role: "data" },
        #{ t: "  ", role: "data" },
        #{ t: round(host.load15, 2), role: "data" },
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
            role: "data",
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
