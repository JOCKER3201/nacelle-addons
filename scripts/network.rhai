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
        // The addresses are instrument data: monospace, tabular, right
        // where the images put their numerals.
        rows([
            ["IPV4", ipv4, ipv4_sev],
            ["PING", ping, ping_sev],
        ], #{ value_role: "data" }),
    ]
}
