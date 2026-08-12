// label: MEMORY
// ref_h: 12.0
// min_h: 9.0
// slot: left
// order: 4
// weight: 18
fn draw() {
    let out = [
        // The heading moves to the host's band (u2 §2.6).
        title("MEMORY"),
        // The nameless row becomes the header line above the matrix
        // (u2 §2.6): its label STAYS empty — u2 §6.3's default — and its
        // value is the same used / total string as always, set small so
        // it reads as the matrix's caption.
        rows([["", bytes(host.mem_used) + " / " + bytes(host.mem_total)]],
             #{ value_role: "caption" }),
        dots(host.mem_fraction),
    ];
    // No swap configured means no swap meter, rather than an empty bar.
    if host.swap_total > 0.0 {
        // The script's judgement of the data (§5.10): swap mostly full
        // is worth a warning, nearly full is critical, and an ordinary
        // reading carries no severity at all. The colours are the
        // theme's.
        let sev = if host.swap_fraction >= 0.9 {
            "critical"
        } else if host.swap_fraction >= 0.6 {
            "warning"
        } else {
            ""
        };
        out.push(meter("SWAP", host.swap_fraction, bytes(host.swap_used),
                       #{ severity: sev }));
    }
    out
}
