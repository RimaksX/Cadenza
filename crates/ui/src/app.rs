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

/// How often the spectrum is read.
///
/// Twenty a second, under the ceiling of thirty PROJECT_MASTER 2.9 sets. The
/// cap is not the cost: reading the spectrum is a tenth of a per cent of one
/// core, and *drawing* it is eight per cent, because every change repaints the
/// window. Twenty is where a row of bars still moves like sound and the price
/// is a third off.
///
/// Nothing is read while nothing is playing, and while nothing is read nothing
/// is copied out of the audio callback either.
const FRAME: Duration = Duration::from_millis(50);

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

    // The visualiser has a clock of its own: section 2.9 caps it at thirty a
    // second, and the transport above is happy at four. Kept alive alongside
    // that one, for the same reason.
    let frames = Timer::default();
    frames.start(TimerMode::Repeated, FRAME, {
        let controller = Rc::clone(&controller);
        move || controller.refresh_spectrum()
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

    window.on_play_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.play_playlist(&id)
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

    window.on_add_to_playlist({
        let controller = Rc::clone(controller);
        move |track, playlist| controller.add_to_playlist(&track, &playlist)
    });

    window.on_create_playlist_with({
        let controller = Rc::clone(controller);
        move |track, name| controller.create_playlist_with(&track, &name)
    });

    window.on_remove_from_playlist({
        let controller = Rc::clone(controller);
        move |position| controller.remove_from_playlist(position)
    });

    window.on_remove_from_library({
        let controller = Rc::clone(controller);
        move |id| controller.remove_from_library(&id)
    });

    window.on_showing({
        let controller = Rc::clone(controller);
        move |name| controller.showing(name.as_str())
    });

    window.on_edit_track({
        let controller = Rc::clone(controller);
        move |id| controller.edit_track(id.as_str())
    });

    window.on_save_track({
        let controller = Rc::clone(controller);
        move |id, title, artist, album| {
            controller.save_track(id.as_str(), title.as_str(), artist.as_str(), album.as_str());
        }
    });

    window.on_switch_profile({
        let controller = Rc::clone(controller);
        move |id| controller.switch_profile(id.as_str())
    });

    window.on_create_profile({
        let controller = Rc::clone(controller);
        move |name| controller.create_profile(name.as_str())
    });

    window.on_start_radio({
        let controller = Rc::clone(controller);
        move |id| controller.start_radio(id.as_str())
    });

    window.on_judge_radio({
        let controller = Rc::clone(controller);
        move |like| controller.judge_radio(like)
    });

    window.on_clear_queue({
        let controller = Rc::clone(controller);
        move || controller.clear_queue()
    });

    window.on_search({
        let controller = Rc::clone(controller);
        move |query| controller.search(&query)
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

    window.on_set_eq_mode({
        let controller = Rc::clone(controller);
        move |advanced| controller.set_eq_mode(advanced)
    });

    window.on_set_eq_simple({
        let controller = Rc::clone(controller);
        move |which, value| controller.set_eq_simple(which, value)
    });

    window.on_move_eq_band({
        let controller = Rc::clone(controller);
        move |index, y| controller.move_eq_band(index, y)
    });

    window.on_tune_eq_band({
        let controller = Rc::clone(controller);
        move |index, direction| controller.tune_eq_band(index, direction)
    });

    window.on_select_eq_band({
        let controller = Rc::clone(controller);
        move |index| controller.select_eq_band(index)
    });

    window.on_settle_eq({
        let controller = Rc::clone(controller);
        move || controller.settle_eq()
    });

    window.on_widen_eq_band({
        let controller = Rc::clone(controller);
        move |index, step| controller.widen_eq_band(index, step)
    });

    window.on_pick_eq_preset({
        let controller = Rc::clone(controller);
        move |id| controller.pick_eq_preset(&id)
    });

    window.on_save_eq_preset({
        let controller = Rc::clone(controller);
        move |name| controller.save_eq_preset(&name)
    });

    window.on_rename_eq_preset({
        let controller = Rc::clone(controller);
        move |id, name| controller.rename_eq_preset(&id, &name)
    });

    window.on_delete_eq_preset({
        let controller = Rc::clone(controller);
        move |id| controller.delete_eq_preset(&id)
    });

    window.on_set_theme({
        let controller = Rc::clone(controller);
        move |dark| controller.set_theme(dark)
    });

    window.on_set_crossfade({
        let controller = Rc::clone(controller);
        move |on, seconds| controller.set_crossfade(on, seconds)
    });

    window.on_set_history({
        let controller = Rc::clone(controller);
        move |keep| controller.set_history(keep)
    });

    window.on_add_folder({
        let controller = Rc::clone(controller);
        move || controller.add_folder()
    });

    window.on_use_suggested_folder({
        let controller = Rc::clone(controller);
        move || controller.use_suggested_folder()
    });

    window.on_remove_folder({
        let controller = Rc::clone(controller);
        move |id| controller.remove_folder(&id)
    });

    window.on_scan_now({
        let controller = Rc::clone(controller);
        move || controller.scan_now()
    });

    window.on_reset_eq({
        let controller = Rc::clone(controller);
        move || controller.reset_eq()
    });
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
