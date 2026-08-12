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
        // value is the same used / total string as always.
        //
        // It used to name the `caption` role to read as the matrix's
        // caption, which set this panel's headline figure at 1.77u under
        // its own swap readout at 3.25u: the less important number was
        // twice the size of the more important one. Both are values of a
        // rows line now, so both are one size, and which size that is is
        // script.rows_value_role's to say.
        rows([["", bytes(host.mem_used) + " / " + bytes(host.mem_total)]]),
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
