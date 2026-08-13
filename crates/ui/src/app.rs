//! Building the window and running the event loop.

use std::rc::Rc;
use std::time::Duration;

use cadenza_core::{CoreError, Result};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::controller::Controller;
use crate::{AppWindow, UiServices};

/// How often the position and transport state are re-read.
///
/// Four times a second: fast enough that a progress line does not visibly step,
/// slow enough to be free. The engine is the clock; this only asks it what time
/// it is (PROJECT_MASTER 2.9 caps the far more expensive visualiser at 30 Hz).
const TICK: Duration = Duration::from_millis(250);

/// Opens the window and blocks until it closes.
pub fn run(services: UiServices) -> Result<()> {
    let window = AppWindow::new().map_err(|err| CoreError::Invalid {
        field: "window",
        reason: format!("the interface could not be created: {err}"),
    })?;

    let controller = Rc::new(Controller::new(services, window.as_weak()));
    controller.refresh_all();

    wire(&window, &controller);

    // Kept alive for as long as the loop runs: a dropped timer stops ticking,
    // and the progress line would freeze while the music kept playing.
    let ticker = Timer::default();
    ticker.start(TimerMode::Repeated, TICK, {
        let controller = Rc::clone(&controller);
        move || {
            // Advance first, so that a track which ran out between two ticks is
            // replaced before the bar is drawn holding it.
            controller.poll_queue();
            controller.refresh_player();
        }
    });

    window.run().map_err(|err| CoreError::Invalid {
        field: "window",
        reason: format!("the interface stopped: {err}"),
    })
}

/// Connects every callback the markup declares.
///
/// One place, so that a callback the window offers and nobody answers is
/// visible as an absence here rather than as silence at runtime.
fn wire(window: &AppWindow, controller: &Rc<Controller>) {
    wire_window_controls(window);

    window.on_play({
        let controller = Rc::clone(controller);
        move |id| controller.play(&id)
    });

    window.on_enqueue({
        let controller = Rc::clone(controller);
        move |id| controller.enqueue(&id)
    });

    window.on_play_queued({
        let controller = Rc::clone(controller);
        move |position| controller.play_queued(position)
    });

    window.on_remove_from_queue({
        let controller = Rc::clone(controller);
        move |position| controller.remove_from_queue(position)
    });

    window.on_open_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.open_playlist(&id)
    });

    window.on_create_playlist({
        let controller = Rc::clone(controller);
        move |name| controller.create_playlist(&name)
    });

    window.on_rename_playlist({
        let controller = Rc::clone(controller);
        move |id, name| controller.rename_playlist(&id, &name)
    });

    window.on_delete_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.delete_playlist(&id)
    });

    window.on_play_from_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.play_from_playlist(&id)
    });

    window.on_toggle_play({
        let controller = Rc::clone(controller);
        move || controller.toggle_play()
    });

    window.on_seek({
        let controller = Rc::clone(controller);
        move |fraction| controller.seek(fraction)
    });

    window.on_set_volume({
        let controller = Rc::clone(controller);
        move |level| controller.set_volume(level)
    });

    window.on_toggle_mute({
        let controller = Rc::clone(controller);
        move || controller.toggle_mute()
    });

    window.on_next({
        let controller = Rc::clone(controller);
        move || controller.next()
    });

    window.on_previous({
        let controller = Rc::clone(controller);
        move || controller.previous()
    });

    window.on_toggle_shuffle({
        let controller = Rc::clone(controller);
        move || controller.toggle_shuffle()
    });

    window.on_cycle_repeat({
        let controller = Rc::clone(controller);
        move || controller.cycle_repeat()
    });

    // No theme callback: the palette is read from the profile at startup and
    // changed from the settings screen, which does not exist yet. Until then
    // `cadenza theme <dark|light>` is the way.
}

/// The three buttons and the drag, which the system frame used to provide.
///
/// None of this reaches the application layer: moving a window is not a use
/// case, it is the window.
fn wire_window_controls(window: &AppWindow) {
    window.on_minimize({
        let handle = window.as_weak();
        move || {
            if let Some(window) = handle.upgrade() {
                window.window().set_minimized(true);
            }
        }
    });

    window.on_toggle_maximize({
        let handle = window.as_weak();
        move || {
            if let Some(window) = handle.upgrade() {
                let maximized = !window.window().is_maximized();
                window.window().set_maximized(maximized);
                window.set_maximized(maximized);
            }
        }
    });

    window.on_close_window({
        let handle = window.as_weak();
        move || {
            if let Some(window) = handle.upgrade() {
                // Hiding the last window ends the event loop, which returns
                // from `run` and drops the engine and the pool in order.
                let _ = window.hide();
            }
        }
    });

    window.on_drag_window({
        let handle = window.as_weak();
        move |dx, dy| {
            let Some(window) = handle.upgrade() else {
                return;
            };
            // A maximised window that is dragged should come loose, the way
            // every other window on the platform does.
            if window.window().is_maximized() {
                window.window().set_maximized(false);
                window.set_maximized(false);
                return;
            }

            // The deltas arrive in logical pixels because that is what the
            // markup measures in; the position is physical.
            let scale = window.window().scale_factor();
            let position = window.window().position();
            window.window().set_position(slint::PhysicalPosition::new(
                position.x + (dx * scale) as i32,
                position.y + (dy * scale) as i32,
            ));
        }
    });
}
