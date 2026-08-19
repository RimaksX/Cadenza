//! Files dragged onto the window from outside it.
//!
//! Slint's own drag and drop is internal: a `DragArea` hands a string to a
//! `DropArea` in the same window. Files coming from the desktop are a different
//! thing, and its winit backend does not look at them — winit reports
//! `HoveredFile` and `DroppedFile`, and nothing downstream reads either.
//!
//! So they are collected one level lower, from the event loop itself, and left
//! in a box for the window to find on its next frame. Nothing here touches the
//! interface: the handler runs inside the loop before Slint sees the event,
//! which is the worst possible place to start a repaint from.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cadenza_core::{CoreError, Result};
use i_slint_backend_winit::winit::event::WindowEvent;
use i_slint_backend_winit::{Backend, CustomApplicationHandler, EventResult};

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

/// Watches the event loop for files, and does nothing else with it.
struct FileDrops {
    drops: DropBox,
}

impl CustomApplicationHandler for FileDrops {
    fn window_event(
        &mut self,
        _event_loop: &i_slint_backend_winit::winit::event_loop::ActiveEventLoop,
        _window_id: i_slint_backend_winit::winit::window::WindowId,
        _winit_window: Option<&i_slint_backend_winit::winit::window::Window>,
        _slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        let mut carried = self.drops.carried();

        match event {
            WindowEvent::HoveredFile(_) => carried.hovering += 1,
            WindowEvent::HoveredFileCancelled => carried.hovering = 0,
            WindowEvent::DroppedFile(path) => {
                // No cancellation follows a drop, so the count is cleared here.
                // The last file of a batch arrives with the others, and the
                // window reads them together on its next frame.
                carried.hovering = 0;
                carried.dropped.push(path.clone());
            }
            _ => {}
        }

        // Everything is passed on untouched. This handler observes; it does not
        // take events away from the interface that is drawn on top of it.
        EventResult::Propagate
    }
}

/// Puts the drop-aware backend in place, and hands back what it fills.
///
/// Must run before the first window is created: a platform can only be chosen
/// while there is nothing yet drawn by one.
pub fn install() -> Result<DropBox> {
    let drops = DropBox::default();

    let backend = Backend::builder()
        .with_custom_application_handler(Box::new(FileDrops {
            drops: drops.clone(),
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
