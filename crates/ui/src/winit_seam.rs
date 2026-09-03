//! The two things winit knows that Slint does not look at.
//!
//! Slint's winit backend publishes a seam — a handler that sees every window
//! event before the interface does. Cadenza uses it for exactly two things, and
//! both are cases where the information exists one level down and is thrown
//! away one level up.
//!
//! **Files dragged onto the window from outside it.** Slint's own drag and drop
//! is internal: a `DragArea` hands a string to a `DropArea` in the same window.
//! Files coming from the desktop are a different thing — winit reports
//! `HoveredFile` and `DroppedFile`, and nothing downstream reads either. They
//! are collected here and left in a box for the window to find on its next
//! frame. Nothing touches the interface from in here: this runs inside the loop,
//! which is the worst possible place to start a repaint from.
//!
//! **Ctrl+V on a keyboard that is not American.** Slint decides that a key
//! combination is Paste by comparing the *character produced* against `"v"`
//! (`i-slint-core`, `input.rs`). On a Russian layout the V key produces `м`, on
//! a Greek one `ω`, and the comparison fails — so paste, copy, cut and
//! select-all are all dead for anybody not typing in Latin. What winit reports
//! and Slint's key event cannot carry is the *physical* key, which is `KeyV`
//! whatever is printed on it. So those six combinations are translated here and
//! handed on as the letter Slint is looking for.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cadenza_core::{CoreError, Result};
use i_slint_backend_winit::winit::event::{ElementState, WindowEvent};
use i_slint_backend_winit::winit::keyboard::{KeyCode, PhysicalKey};
use i_slint_backend_winit::{Backend, CustomApplicationHandler, EventResult};

/// The editing shortcuts, by the key's place rather than by its legend.
///
/// Six, which is every combination `i-slint-core` recognises for a text field.
/// Undo and redo are here for the same reason as paste: they are matched
/// against `"z"` and `"y"`, and a Russian keyboard's Z key produces `я`.
const SHORTCUTS: [(KeyCode, &str); 6] = [
    (KeyCode::KeyA, "a"),
    (KeyCode::KeyC, "c"),
    (KeyCode::KeyV, "v"),
    (KeyCode::KeyX, "x"),
    (KeyCode::KeyY, "y"),
    (KeyCode::KeyZ, "z"),
];

/// The letter Slint expects for a key in that place, if it is one of the six.
fn latin(code: KeyCode) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|(shortcut, _)| *shortcut == code)
        .map(|(_, letter)| *letter)
}

/// What the pointer is carrying over the window, and what it let go of.
#[derive(Default)]
struct Carried {
    /// How many files are being held over the window.
    ///
    /// Counted rather than flagged: one drag of three files is three
    /// `HoveredFile` events and three `DroppedFile`s, and a flag would be turned
    /// off by the first of them.
    hovering: usize,
    /// Paths let go of and not yet collected.
    dropped: Vec<PathBuf>,
}

/// The window's end of it, read once a frame from the event loop.
#[derive(Clone, Default)]
pub struct DropBox(Arc<Mutex<Carried>>);

impl DropBox {
    fn carried(&self) -> MutexGuard<'_, Carried> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// True while something from outside is over the window.
    pub fn hovering(&self) -> bool {
        self.carried().hovering > 0
    }

    /// Everything let go of since the last look.
    pub fn take(&self) -> Vec<PathBuf> {
        std::mem::take(&mut self.carried().dropped)
    }
}

/// Watches the event loop for files and for shortcuts, and nothing else.
struct Seam {
    drops: DropBox,
    /// Whether a control key is down, as winit last reported it.
    ///
    /// Kept rather than asked for: a key event carries no modifier state of its
    /// own, and `ModifiersChanged` is the event that carries it.
    control: bool,
}

impl CustomApplicationHandler for Seam {
    fn window_event(
        &mut self,
        _event_loop: &i_slint_backend_winit::winit::event_loop::ActiveEventLoop,
        _window_id: i_slint_backend_winit::winit::window::WindowId,
        _winit_window: Option<&i_slint_backend_winit::winit::window::Window>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        match event {
            WindowEvent::ModifiersChanged(state) => {
                self.control = state.state().control_key();
            }

            // Translated on every layout, including the one where it would have
            // worked anyway. Doing it only for the layouts that need it would
            // mean two paths through the same press, and the one nobody here
            // types on would be the untested one.
            WindowEvent::KeyboardInput { event: key, .. } if self.control => {
                if let PhysicalKey::Code(code) = key.physical_key
                    && let Some(letter) = latin(code)
                    && let Some(window) = slint_window
                {
                    window.dispatch_event(match key.state {
                        ElementState::Pressed => slint::platform::WindowEvent::KeyPressed {
                            text: letter.into(),
                        },
                        ElementState::Released => slint::platform::WindowEvent::KeyReleased {
                            text: letter.into(),
                        },
                    });

                    // The only event this handler ever takes away. What it
                    // replaces it with is the same press, spelled the way the
                    // layer above reads.
                    return EventResult::PreventDefault;
                }
            }

            WindowEvent::HoveredFile(_) => self.drops.carried().hovering += 1,
            WindowEvent::HoveredFileCancelled => self.drops.carried().hovering = 0,
            WindowEvent::DroppedFile(path) => {
                // No cancellation follows a drop, so the count is cleared here.
                // The last file of a batch arrives with the others, and the
                // window reads them together on its next frame.
                let mut carried = self.drops.carried();
                carried.hovering = 0;
                carried.dropped.push(path.clone());
            }
            _ => {}
        }

        // Everything else is passed on untouched. This handler observes; it does
        // not take events away from the interface drawn on top of it.
        EventResult::Propagate
    }
}

/// Puts the seam in place, and hands back what the file half of it fills.
///
/// Must run before the first window is created: a platform can only be chosen
/// while there is nothing yet drawn by one.
pub fn install() -> Result<DropBox> {
    let drops = DropBox::default();

    let backend = Backend::builder()
        .with_custom_application_handler(Box::new(Seam {
            drops: drops.clone(),
            control: false,
        }))
        .build()
        .map_err(|err| CoreError::Invalid {
            field: "window",
            reason: format!("the window backend could not be built: {err}"),
        })?;

    slint::platform::set_platform(Box::new(backend)).map_err(|err| CoreError::Invalid {
        field: "window",
        reason: format!("the window backend could not be installed: {err}"),
    })?;

    Ok(drops)
}

#[cfg(test)]
mod tests {
    use super::{KeyCode, latin};

    #[test]
    fn the_six_editing_shortcuts_are_named_by_place() {
        assert_eq!(latin(KeyCode::KeyV), Some("v"), "paste, whatever it prints");
        assert_eq!(latin(KeyCode::KeyA), Some("a"));
        assert_eq!(
            latin(KeyCode::KeyZ),
            Some("z"),
            "undo, я on a Russian board"
        );

        // Everything else is left alone: Ctrl with any other key is not this
        // handler's business, and a translation table that grew would start
        // taking events the interface wanted.
        assert_eq!(latin(KeyCode::KeyB), None);
        assert_eq!(latin(KeyCode::Enter), None);
    }
}
