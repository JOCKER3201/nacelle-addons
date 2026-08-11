// An instrument strip (u2 §2.2): three cells with hairline dividers,
// labels and values aligned by the theme's columns.align. POWER carries
// the script's judgement of the battery — a severity from the closed
// set, never a colour; AC and a healthy battery read as ok, charging as
// info, a draining battery degrades through warning to critical.
fn draw() {
    let power = if host.battery == () {
        "AC"
    } else if host.charging {
        "" + host.battery + "% +"
    } else {
        "" + host.battery + "%"
    };
    let power_sev = if host.battery == () {
        "ok"
    } else if host.charging {
        "info"
    } else if host.battery < 10 {
        "critical"
    } else if host.battery < 20 {
        "warning"
    } else {
        "ok"
    };
    [columns([
        ["DATE",   upper(host.date)],
        ["UPTIME", uptime(host.uptime)],
        ["POWER",  power, power_sev],
    ], #{ dividers: true })]
}
