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
        move || controller.refresh_player()
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
    window.on_play({
        let controller = Rc::clone(controller);
        move |id| controller.play(&id)
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

    // No theme callback: the palette is read from the profile at startup and
    // changed from the settings screen, which does not exist yet. Until then
    // `cadenza theme <dark|light>` is the way.
}
