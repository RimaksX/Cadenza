//! Cadenza composition root.
//!
//! M0: proves the workspace builds and the binary runs. The real startup sequence
//! (paths, logging, config, wiring, lifecycle, runtime) lands in M3 and M6.

#![forbid(unsafe_code)]

fn main() {
    println!(
        "{} {} (scaffold, milestone M0)",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
}
