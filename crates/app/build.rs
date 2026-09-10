//! Puts the application's icon inside the executable.
//!
//! Not the installer's job. A shortcut can be given an icon by whoever makes
//! the shortcut, but the icon Windows shows for `cadenza.exe` itself — in the
//! folder it was unpacked into, in the taskbar, in the Alt+Tab list, on the
//! window — is a resource compiled into the binary. Without this the program is
//! the blank sheet Windows draws for something it knows nothing about.
//!
//! This is also where the executable's own description and version come from,
//! which is what the file's Properties panel shows.

fn main() {
    println!("cargo::rerun-if-changed=../../resources/branding/cadenza.ico");
    println!("cargo::rerun-if-changed=build.rs");

    // Only Windows has this kind of resource, and this is the only platform
    // Cadenza targets — but the guard keeps a check on
    // another machine from failing on a step that has nothing to do there.
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../resources/branding/cadenza.ico");
    resource.set("FileDescription", "Cadenza — local music player");
    resource.set("ProductName", "Cadenza");
    resource.set("LegalCopyright", "GPL-3.0-only");

    // Loud rather than quiet. A build that silently produced an iconless
    // executable would be found by somebody looking at a taskbar weeks later,
    // and this project has spent enough of this milestone on failures that
    // were swallowed somewhere.
    resource
        .compile()
        .expect("the icon could not be compiled into the executable");
}
