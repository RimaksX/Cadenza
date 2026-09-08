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
use i_slint_backend_winit::winit::dpi::PhysicalPosition;
use i_slint_backend_winit::winit::event::{ElementState, MouseButton, WindowEvent};
use i_slint_backend_winit::winit::keyboard::{KeyCode, PhysicalKey};
use i_slint_backend_winit::winit::window::{CursorIcon, ResizeDirection, Window};
use i_slint_backend_winit::{Backend, CustomApplicationHandler, EventResult};

/// How near the edge counts as the edge, in logical pixels.
///
/// Six: wide enough to be caught without aiming, narrow enough that nothing
/// inside the window is inside it. The system's own frames are about four and
/// are famously hard to hit; this interface keeps its controls well clear of
/// the border, so there is room to be more generous than the platform is.
const EDGE: f64 = 6.0;

/// Which edge or corner a point is on, if it is on one.
///
/// Corners first, because a corner is on two edges, and taking one of them
/// instead is the difference between resizing the window and resizing one side
/// of it.
fn edge_at(x: f64, y: f64, width: f64, height: f64, band: f64) -> Option<ResizeDirection> {
    let west = x <= band;
    let east = x >= width - band;
    let north = y <= band;
    let south = y >= height - band;

    match (north, south, west, east) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (true, _, _, true) => Some(ResizeDirection::NorthEast),
        (_, true, true, _) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (true, ..) => Some(ResizeDirection::North),
        (_, true, ..) => Some(ResizeDirection::South),
        (_, _, true, _) => Some(ResizeDirection::West),
        (_, _, _, true) => Some(ResizeDirection::East),
        _ => None,
    }
}

/// Where the pointer is on the window's frame, or `None` for anywhere else.
///
/// A maximised window has no frame to pull: it is the size of the screen, and
/// the title bar's own button is the way back.
fn edge_under(window: &Window, pointer: PhysicalPosition<f64>) -> Option<ResizeDirection> {
    if window.is_maximized() {
        return None;
    }

    let size = window.inner_size();
    edge_at(
        pointer.x,
        pointer.y,
        f64::from(size.width),
        f64::from(size.height),
        EDGE * window.scale_factor(),
    )
}

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
    /// What the window measured, last time it changed.
    ///
    /// Written here rather than logged from the event loop, which has no log
    /// and should not grow one: the loop records, the tick reports.
    geometry: Option<String>,
    /// Whether that measurement has been handed out yet.
    ///
    /// Kept beside the measurement rather than taken out of it: a reader that
    /// emptied the field would leave the next event with nothing to compare
    /// against, and every event after the first would look like a change. Four
    /// identical lines a second is not a diagnostic, it is a log nobody reads.
    fresh: bool,
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

    /// What the window measured, if it has changed since this was last asked.
    ///
    /// Returned once and then forgotten, so a size that never changes is one
    /// line in the log rather than four a second.
    ///
    /// It exists because a window can be a different size from the surface
    /// drawn into it, and when that happens every click lands on whatever is
    /// drawn above what was pressed, while the bottom of the window falls off
    /// the end. Nothing here can be reproduced on the machine that writes it,
    /// so what is needed from the machine that has it is numbers rather than
    /// impressions (`MASTER_ISSUES` 127).
    pub fn measured(&self) -> Option<String> {
        let mut carried = self.carried();
        if !carried.fresh {
            return None;
        }
        carried.fresh = false;
        carried.geometry.clone()
    }
}

/// Watches the event loop for files, for shortcuts, and for the window's edges.
struct Seam {
    drops: DropBox,
    /// Whether a control key is down, as winit last reported it.
    ///
    /// Kept rather than asked for: a key event carries no modifier state of its
    /// own, and `ModifiersChanged` is the event that carries it.
    control: bool,
    /// Where the pointer was, physically, as winit last reported it.
    ///
    /// Kept for the same reason: a button event says which button and not
    /// where, so a press has to be answered from what the last movement said.
    pointer: PhysicalPosition<f64>,
    /// The edge the cursor is currently drawn for.
    ///
    /// Remembered so the cursor is set when it changes rather than on every
    /// movement, and so leaving the frame puts the arrow back exactly once.
    /// Slint sets the cursor for whatever is under the pointer after this
    /// handler has run, so anything inside the window still wins.
    showing: Option<ResizeDirection>,
}

impl Seam {
    /// Records what the window and the surface inside it each think they are.
    ///
    /// Only when it changes. Both sides are asked, because the fault this is
    /// here to catch is the two of them disagreeing: winit's `inner_size` is
    /// the client area Windows gave us, and Slint's is the surface the
    /// interface was laid out for. When those differ the interface is drawn to
    /// a height the window does not have.
    fn measure(&mut self, winit_window: Option<&Window>, slint_window: Option<&slint::Window>) {
        let (Some(window), Some(slint)) = (winit_window, slint_window) else {
            return;
        };

        let inner = window.inner_size();
        let outer = window.outer_size();
        let position = window.outer_position().unwrap_or_default();
        let surface = slint.size();

        let said = format!(
            "window: client {}x{}, frame {}x{} at {},{}, dpi {:.2}; \
             surface {}x{}, dpi {:.2}",
            inner.width,
            inner.height,
            outer.width,
            outer.height,
            position.x,
            position.y,
            window.scale_factor(),
            surface.width,
            surface.height,
            slint.scale_factor(),
        );

        let mut carried = self.drops.carried();
        if carried.geometry.as_deref() != Some(said.as_str()) {
            carried.geometry = Some(said);
            carried.fresh = true;
        }
    }
}

impl CustomApplicationHandler for Seam {
    fn window_event(
        &mut self,
        _event_loop: &i_slint_backend_winit::winit::event_loop::ActiveEventLoop,
        _window_id: i_slint_backend_winit::winit::window::WindowId,
        winit_window: Option<&Window>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        self.measure(winit_window, slint_window);

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

            // The frame is ours, and this is the one thing a drawn frame does
            // not get for free.
            //
            // An undecorated window on Windows answers every point with
            // "client" — measured rather than assumed: the style still carries
            // `WS_THICKFRAME`, so the system would resize the window, and
            // `WM_NCHITTEST` never says `LEFT`, `BOTTOM` or any other edge, so
            // nothing ever asks it to. The edges were dead, and the note in
            // `app_window.slint` saying the backend kept them alive was wrong
            // (`MASTER_ISSUES` 88).
            //
            // So the edge is found here and handed to the platform's own
            // resize loop — the one that follows the pointer, honours the
            // minimum size and snaps — rather than to arithmetic of our own.
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = *position;

                if let Some(window) = winit_window {
                    let edge = edge_under(window, *position);
                    if edge != self.showing {
                        self.showing = edge;
                        window.set_cursor(edge.map_or(CursorIcon::Default, CursorIcon::from));
                    }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(window) = winit_window
                    && let Some(direction) = edge_under(window, self.pointer)
                {
                    // Ignored on purpose: a platform that cannot start a resize
                    // this way leaves the window the size it is, which is what
                    // it did before there was an edge to pull.
                    let _ = window.drag_resize_window(direction);
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
            pointer: PhysicalPosition::default(),
            showing: None,
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
    use super::{EDGE, KeyCode, ResizeDirection, edge_at, latin};

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

    #[test]
    fn the_frame_is_the_outer_six_pixels() {
        let edge = |x, y| edge_at(x, y, 1180.0, 760.0, EDGE);

        assert_eq!(edge(0.0, 0.0), Some(ResizeDirection::NorthWest));
        assert_eq!(edge(1179.0, 759.0), Some(ResizeDirection::SouthEast));
        assert_eq!(edge(1179.0, 0.0), Some(ResizeDirection::NorthEast));
        assert_eq!(edge(0.0, 759.0), Some(ResizeDirection::SouthWest));

        assert_eq!(edge(600.0, 2.0), Some(ResizeDirection::North));
        assert_eq!(edge(600.0, 758.0), Some(ResizeDirection::South));
        assert_eq!(edge(3.0, 400.0), Some(ResizeDirection::West));
        assert_eq!(edge(1177.0, 400.0), Some(ResizeDirection::East));

        // A corner is on two edges and it is the corner that is meant: a press
        // seven pixels along the top is the top, one pixel further in is both.
        assert_eq!(edge(7.0, 1.0), Some(ResizeDirection::North));
        assert_eq!(edge(5.0, 1.0), Some(ResizeDirection::NorthWest));

        // And everything the interface is drawn in is not the frame.
        assert_eq!(edge(7.0, 7.0), None);
        assert_eq!(edge(590.0, 380.0), None);
        assert_eq!(edge(1173.0, 753.0), None);
    }
}
