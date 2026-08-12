// label: CLOCK
// ref_h: 7.0
// min_h: 5.5
// slot: right
// order: 4
// weight: 8
// The separator blinks by ALPHA, not by glyph substitution: each colon
// keeps its advance and the runs element drives its visibility from
// motion.value_blink, so the line never re-measures and cannot jitter
// (u2 §2.1, I13). Reduced motion freezes the colon fully visible.
fn draw() {
    let hh = if host.hour < 10 { "0" } else { "" } + host.hour;
    let mm = if host.minute < 10 { "0" } else { "" } + host.minute;
    let ss = if host.second < 10 { "0" } else { "" } + host.second;
    // "clock" and "date" are KINDS, not type roles: this widget says the
    // runs are the running time and the line under them is its date, and
    // the theme's kind_clock_role / kind_date_role say what those are set
    // in. Both land on tabular roles, which is what stops the line
    // changing width when a 1 takes the place of a 2.
    [
        runs([
            #{ t: hh, role: "clock" },
            #{ t: ":", role: "clock", blink: "value_blink" },
            #{ t: mm, role: "clock" },
            #{ t: ":", role: "clock", blink: "value_blink" },
            #{ t: ss, role: "clock" },
        ], "center"),
        text(upper(host.date_long), "center", #{ role: "date" }),
    ]
}
