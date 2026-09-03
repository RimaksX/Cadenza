//! Saying something when there is no window to say it in.
//!
//! Cadenza is built for the windows subsystem, which means it has no console:
//! `eprintln!` writes to a handle nobody is holding. Everything the listener
//! needs to be told normally reaches them through the interface, and everything
//! nobody can be told goes to the log — but between starting and the window
//! opening there is a stretch with neither, and a failure there would show as
//! an icon that was clicked and did nothing.
//!
//! So: exactly two occasions. The application cannot start, and the application
//! has stopped in a way it did not plan for. Anything else belongs in the
//! window or the log.

/// Shows a message the listener cannot miss, and waits for them to dismiss it.
///
/// `rfd` again, which is already here for the folder picker — on Windows it is
/// a thin wrapper over the task dialog. A dialog rather than a line of text
/// because the alternative is silence.
pub fn fatal(message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Cadenza")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
