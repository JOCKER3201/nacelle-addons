// label: NETWORK
// ref_h: 8.0
// min_h: 8.0
// slot: right
// order: 3
// weight: 10
fn draw() {
    // The script judges the DATA, never the look (§5.10): online is
    // well, offline is the offline severity, and an absent reading is
    // unknown — read as absent, never as zero and never as ok. What any
    // of that looks like is the theme's business.
    let state = if host.online { "ONLINE" } else { "OFFLINE" };
    let state_sev = if host.online { "ok" } else { "offline" };
    let ipv4 = if host.ipv4 == () { "--" } else { host.ipv4 };
    let ipv4_sev = if host.ipv4 == () { "unknown" } else { "" };
    let ping = if host.ping == () { "--" } else { "" + host.ping + "ms" };
    let ping_sev = if host.ping == () { "unknown" } else { "" };
    [
        // Heading and interface name move to the host's band (u2 §2.8).
        title("NETWORK", upper(host.iface)),
        // STATE is a badge, not a bare word (u2 §2.8): the status pill
        // of the reference images, on the label's own line, carrying
        // exactly the string the old rows line showed.
        badge(state, #{ label: "STATE", severity: state_sev }),
        // Two rows with a label and a reading, and that is the whole of
        // what this widget says about them. It used to add
        // #{ value_role: "data" } to hold the address still — the `data`
        // role was the only one carrying tabular figures a script could
        // reach — and paid for it in size: 1.87u where every other panel
        // sets a value at 3.25u, so the same reading was 42% shorter
        // here and the value role is 74% taller than what this widget
        // took. type.value.tabular does that job now, so the address
        // stands still AND reads at the one value size.
        rows([
            ["IPV4", ipv4, ipv4_sev],
            ["PING", ping, ping_sev],
        ]),
    ]
}
