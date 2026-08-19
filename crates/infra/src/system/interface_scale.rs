//! Telling the window toolkit how large to draw, before it draws anything.

/// The variable Slint reads when it creates a window.
const SLINT_SCALE: &str = "SLINT_SCALE_FACTOR";

/// Fixes the scale the interface is drawn at, for this run.
///
/// Slint has no way to change it afterwards that survives: dispatching
/// `ScaleFactorChanged` at the window is accepted and then overwritten when the
/// window is shown, which was measured twice before this was written
/// (`MASTER_ISSUES` 64). What does work is the variable the backend reads while
/// it is building the window — so the choice is made before the interface
/// exists, and a listener who changes it sees it at the next start.
///
/// The factor is absolute: it replaces the display's own scale rather than
/// multiplying it, so the caller is the one that has to multiply.
///
/// # Safety
///
/// `set_var` is unsafe because another thread reading the environment at the
/// same moment is undefined. This is called from `main`, before any thread of
/// this application exists and before the window toolkit is touched — which is
/// the one place in a program where it is not.
pub fn force(factor: f32) {
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }

    // SAFETY: main, single-threaded, before the audio engine, the analyser, the
    // watcher or the window have been created.
    unsafe {
        std::env::set_var(SLINT_SCALE, format!("{factor}"));
    }
}

/// Whether this run is drawing at a scale of somebody's choosing.
///
/// What the window asks before writing down the display's own scale: with an
/// override in force, what the window reports is our number rather than the
/// display's, and storing that would multiply the choice by itself on the next
/// start.
#[must_use]
pub fn is_forced() -> bool {
    std::env::var(SLINT_SCALE).is_ok()
}
