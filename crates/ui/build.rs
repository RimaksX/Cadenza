//! Compiles the Slint markup into Rust.
//!
//! Only the root file is named: everything it imports is followed, and cargo is
//! told to rerun when any of them changes.

fn main() {
    slint_build::compile("slint/app_window.slint").expect("the Slint markup should compile");
}
