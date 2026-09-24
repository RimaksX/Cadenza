//! Building the window and running the event loop.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cadenza_core::domain::ports::event_bus::DomainEvent;
use cadenza_core::domain::ports::log::{LogLevel, LogPort};
use cadenza_core::domain::settings::SideColumn;
use cadenza_core::{CoreError, Result};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, Timer, TimerMode};

use crate::controller::{Controller, PickedIn};
use crate::{
    AppWindow, Equaliser, Fetch, Home, Library, Player, Playlists, Queue, Radio, Settings, Shelves,
    Transfer, UiServices,
};

/// How often the position and transport state are re-read.
///
/// Four times a second: fast enough that a progress line does not visibly step,
/// slow enough to be free. The engine is the clock; this only asks it what time
/// it is.
const TICK: Duration = Duration::from_millis(250);

/// How often the pointer is asked what it is carrying.
///
/// Twenty a second. This answers a hand, not a clock: somebody holding a file
/// over a window that takes a quarter of a second to admit it reaches for the
/// title bar instead. Asking costs an atomic read.
const FRAME: Duration = Duration::from_millis(50);

/// Opens the window and blocks until it closes.
pub fn run(services: UiServices) -> Result<()> {
    // Before the window, because a platform can only be chosen while nothing has
    // been drawn by one. This is also what puts files dropped from the desktop
    // within reach at all.
    let drops = crate::winit_seam::install()?;

    let window = AppWindow::new().map_err(|err| CoreError::Invalid {
        field: "window",
        reason: format!("the interface could not be created: {err}"),
    })?;

    // Something changed the library with nobody looking at it: the watcher
    // noticing a file, or a scan running behind the window. The handler is
    // called on whichever thread published, so it does the only thing that is
    // safe from there — raises a flag. The tick below already runs on the event
    // loop, and reading the database is its job rather than the watcher's.
    //
    // A flag rather than a queue of events, because a copied album publishes one
    // change per file: five hundred of them and one of them ask the window for
    // exactly the same thing.
    let changed = Arc::new(AtomicBool::new(false));
    services.events.subscribe(Box::new({
        let changed = Arc::clone(&changed);
        move |event| {
            if matches!(
                event,
                DomainEvent::LibraryChanged | DomainEvent::ReviewPending
            ) {
                changed.store(true, Ordering::Relaxed);
            }
        }
    }));

    // Taken before the services are handed to the controller, which owns them
    // from here on.
    let log: Arc<dyn LogPort> = Arc::clone(&services.log);

    let controller = Rc::new(Controller::new(services, window.as_weak()));
    controller.refresh_all();
    controller.restore_placement();

    wire(&window, &controller);

    keep_on_screen(window.as_weak());

    // The system's own ways of closing - Alt+F4, the taskbar - keep the place
    // as the title bar's button does.
    window.window().on_close_requested({
        let controller = Rc::clone(&controller);
        move || {
            controller.keep_placement();
            slint::CloseRequestResponse::HideWindow
        }
    });

    // Kept alive for as long as the loop runs: a dropped timer stops ticking,
    // and the progress line would freeze while the music kept playing.
    let ticker = Timer::default();
    ticker.start(TimerMode::Repeated, TICK, {
        let controller = Rc::clone(&controller);
        let drops = drops.clone();
        let log = Arc::clone(&log);
        move || {
            // Advance first, so that a track which ran out between two ticks is
            // replaced before the bar is drawn holding it.
            controller.poll_queue();
            controller.tick_sleep();
            controller.refresh_player();

            if changed.swap(false, Ordering::Relaxed) {
                controller.refresh_after_change();
            }

            // A download running on its own thread, read the same way the
            // watcher is: nothing off the event loop touches the window.
            controller.poll_fetch();

            // And what the window measured, when it measured something new.
            // Once per change: this is four times a second, and a window that
            // is not being dragged reports nothing.
            if let Some(said) = drops.measured() {
                log.write(LogLevel::Info, &said);
            }
        }
    });

    // A clock of its own, because a drop answers a hand rather than a
    // playhead: twenty a second where the transport above is happy at four.
    // Kept alive alongside that one, for the same reason.
    let frames = Timer::default();
    frames.start(TimerMode::Repeated, FRAME, {
        let controller = Rc::clone(&controller);
        move || {
            controller.carrying_files(drops.hovering());

            // Whatever was being carried inside the window was let go of.
            if drops.released() {
                controller.carry_ended();
            }

            let dropped = drops.take();
            if !dropped.is_empty() {
                controller.accept_drop(dropped);
            }
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
    wire_window_controls(window, controller);

    window.global::<Library>().on_play({
        let controller = Rc::clone(controller);
        move |id| controller.play(&id)
    });

    window.global::<Queue>().on_enqueue({
        let controller = Rc::clone(controller);
        move |id, next| controller.enqueue(&id, next)
    });

    window.global::<Queue>().on_open_source({
        let controller = Rc::clone(controller);
        move || controller.open_source().into()
    });

    window.global::<Player>().on_show_artist({
        let controller = Rc::clone(controller);
        move || controller.show_playing_artist()
    });

    window.global::<Queue>().on_play_queued({
        let controller = Rc::clone(controller);
        move |position| controller.play_queued(position)
    });

    window.global::<Queue>().on_remove_from_queue({
        let controller = Rc::clone(controller);
        move |position| controller.remove_from_queue(position)
    });

    window.global::<Playlists>().on_open_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.open_playlist(&id)
    });

    window.global::<Playlists>().on_play_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.play_playlist(&id)
    });

    window.global::<Playlists>().on_shuffle_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.shuffle_playlist(&id)
    });

    window.global::<Playlists>().on_create_playlist({
        let controller = Rc::clone(controller);
        move |name| controller.create_playlist(&name)
    });

    window.global::<Playlists>().on_rename_playlist({
        let controller = Rc::clone(controller);
        move |id, name| controller.rename_playlist(&id, &name)
    });

    window.global::<Playlists>().on_delete_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.delete_playlist(&id)
    });

    window.global::<Fetch>().on_link_typed({
        let controller = Rc::clone(controller);
        move || controller.link_typed()
    });

    window.global::<Player>().on_toggle_favourite({
        let controller = Rc::clone(controller);
        move || controller.toggle_favourite()
    });

    window.global::<Playlists>().on_add_to_playlist({
        let controller = Rc::clone(controller);
        move |track, playlist| controller.add_to_playlist(&track, &playlist)
    });

    window.global::<Playlists>().on_create_playlist_with({
        let controller = Rc::clone(controller);
        move |track, name| controller.create_playlist_with(&track, &name)
    });

    window.global::<Playlists>().on_remove_from_playlist({
        let controller = Rc::clone(controller);
        move |position| controller.remove_from_playlist(position)
    });

    // What a dragged row carries: opaque to the markup, which only moves it
    // from a row to the row it lands on.
    window.global::<Transfer>().on_of_track(|id| {
        let mut payload = slint::DataTransfer::default();
        payload.set_plain_text(id);
        payload
    });

    window.global::<Settings>().on_set_ui_scale({
        let controller = Rc::clone(controller);
        move |percent| controller.set_interface_scale(percent)
    });

    window.on_toggle_sidebar({
        let controller = Rc::clone(controller);
        move || controller.toggle_fold(SideColumn::Navigation)
    });

    window.on_toggle_panel({
        let controller = Rc::clone(controller);
        move || controller.toggle_fold(SideColumn::NowPlaying)
    });

    window.global::<Settings>().on_set_language({
        let controller = Rc::clone(controller);
        move |tag| controller.set_language(&tag)
    });

    window.global::<Library>().on_choose_cover({
        let controller = Rc::clone(controller);
        move |id| controller.choose_cover(&id)
    });

    window.global::<Library>().on_clear_cover({
        let controller = Rc::clone(controller);
        move |id| controller.clear_cover(&id)
    });

    window.global::<Playlists>().on_choose_playlist_cover({
        let controller = Rc::clone(controller);
        move |id| controller.choose_playlist_cover(&id)
    });

    window.global::<Playlists>().on_clear_playlist_cover({
        let controller = Rc::clone(controller);
        move |id| controller.clear_playlist_cover(&id)
    });

    window.global::<Library>().on_remove_from_library({
        let controller = Rc::clone(controller);
        move |id| controller.remove_from_library(&id)
    });

    window.on_showing({
        let controller = Rc::clone(controller);
        move |section| controller.showing(section)
    });

    window.global::<Library>().on_decide_review({
        let controller = Rc::clone(controller);
        move |id, choice| controller.decide_review(id.as_str(), choice)
    });

    window.global::<Library>().on_edit_track({
        let controller = Rc::clone(controller);
        move |id| controller.edit_track(id.as_str())
    });

    window.global::<Library>().on_save_track({
        let controller = Rc::clone(controller);
        move |id, title, artist, album| {
            controller.save_track(id.as_str(), title.as_str(), artist.as_str(), album.as_str());
        }
    });

    window.global::<Radio>().on_start({
        let controller = Rc::clone(controller);
        move |id| controller.start_radio(id.as_str())
    });

    window.global::<Radio>().on_judge({
        let controller = Rc::clone(controller);
        move |like| controller.judge_radio(like)
    });

    window.global::<Queue>().on_clear_queue({
        let controller = Rc::clone(controller);
        move || controller.clear_queue()
    });

    window.global::<Library>().on_search({
        let controller = Rc::clone(controller);
        move |query| controller.search(&query)
    });

    window.global::<Fetch>().on_fetch({
        let controller = Rc::clone(controller);
        move |link| controller.fetch_from_link(&link)
    });

    window.global::<Fetch>().on_fetch_playlist({
        let controller = Rc::clone(controller);
        move |link| controller.fetch_playlist(&link)
    });

    window.global::<Fetch>().on_stop_fetch({
        let controller = Rc::clone(controller);
        move || controller.stop_fetch()
    });

    window.global::<Fetch>().on_fix_fetch({
        let controller = Rc::clone(controller);
        move || controller.fix_fetch()
    });

    window.global::<Fetch>().on_make_local_folder({
        let controller = Rc::clone(controller);
        move || controller.make_local_folder()
    });

    window.global::<Playlists>().on_play_from_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.play_from_playlist(&id)
    });

    window.global::<Player>().on_toggle_play({
        let controller = Rc::clone(controller);
        move || controller.toggle_play()
    });

    window.global::<Player>().on_seek({
        let controller = Rc::clone(controller);
        move |fraction| controller.seek(fraction)
    });

    window.global::<Player>().on_set_volume({
        let controller = Rc::clone(controller);
        move |level| controller.set_volume(level)
    });

    window.global::<Player>().on_toggle_mute({
        let controller = Rc::clone(controller);
        move || controller.toggle_mute()
    });

    window.global::<Home>().on_play_random_track({
        let controller = Rc::clone(controller);
        move || controller.play_random_track()
    });

    window.global::<Home>().on_play_random_playlist({
        let controller = Rc::clone(controller);
        move || controller.play_random_playlist()
    });

    window.global::<Queue>().on_play_following({
        let controller = Rc::clone(controller);
        move |id| controller.play_following(&id)
    });

    window.global::<Home>().on_clear_favourites({
        let controller = Rc::clone(controller);
        move || controller.clear_favourites()
    });

    window.global::<Player>().on_set_speed({
        let controller = Rc::clone(controller);
        move |percent| controller.set_speed(percent)
    });

    window.global::<Settings>().on_set_normalise({
        let controller = Rc::clone(controller);
        move |enabled| controller.set_normalise(enabled)
    });

    window.global::<Player>().on_set_sleep({
        let controller = Rc::clone(controller);
        move |choice| controller.set_sleep(&choice)
    });

    window.global::<Player>().on_next({
        let controller = Rc::clone(controller);
        move || controller.next()
    });

    window.global::<Player>().on_previous({
        let controller = Rc::clone(controller);
        move || controller.previous()
    });

    window.global::<Player>().on_toggle_shuffle({
        let controller = Rc::clone(controller);
        move || controller.toggle_shuffle()
    });

    window.global::<Player>().on_cycle_repeat({
        let controller = Rc::clone(controller);
        move || controller.cycle_repeat()
    });

    window.global::<Equaliser>().on_set_mode({
        let controller = Rc::clone(controller);
        move |advanced| controller.set_eq_mode(advanced)
    });

    window.global::<Equaliser>().on_set_simple({
        let controller = Rc::clone(controller);
        move |which, value| controller.set_eq_simple(which, value)
    });

    window.global::<Equaliser>().on_move_band({
        let controller = Rc::clone(controller);
        move |index, y| controller.move_eq_band(index, y)
    });

    window.global::<Equaliser>().on_tune_band({
        let controller = Rc::clone(controller);
        move |index, direction| controller.tune_eq_band(index, direction)
    });

    window.global::<Equaliser>().on_select_band({
        let controller = Rc::clone(controller);
        move |index| controller.select_eq_band(index)
    });

    window.global::<Equaliser>().on_settle({
        let controller = Rc::clone(controller);
        move || controller.settle_eq()
    });

    window.global::<Equaliser>().on_widen_band({
        let controller = Rc::clone(controller);
        move |index, step| controller.widen_eq_band(index, step)
    });

    window.global::<Equaliser>().on_pick_preset({
        let controller = Rc::clone(controller);
        move |id| controller.pick_eq_preset(&id)
    });

    window.global::<Equaliser>().on_save_preset({
        let controller = Rc::clone(controller);
        move |name| controller.save_eq_preset(&name)
    });

    window.global::<Equaliser>().on_rename_preset({
        let controller = Rc::clone(controller);
        move |id, name| controller.rename_eq_preset(&id, &name)
    });

    window.global::<Equaliser>().on_delete_preset({
        let controller = Rc::clone(controller);
        move |id| controller.delete_eq_preset(&id)
    });

    window.global::<Settings>().on_set_theme({
        let controller = Rc::clone(controller);
        move |dark| controller.set_theme(dark)
    });

    window.global::<Settings>().on_set_crossfade({
        let controller = Rc::clone(controller);
        move |on, seconds| controller.set_crossfade(on, seconds)
    });

    window.global::<Settings>().on_set_panel_rail({
        let controller = Rc::clone(controller);
        move |keep| controller.set_panel_rail(keep)
    });

    window.global::<Settings>().on_set_history({
        let controller = Rc::clone(controller);
        move |keep| controller.set_history(keep)
    });

    window.global::<Settings>().on_add_folder({
        let controller = Rc::clone(controller);
        move || controller.add_folder()
    });

    window.global::<Settings>().on_use_suggested_folder({
        let controller = Rc::clone(controller);
        move || controller.use_suggested_folder()
    });

    window.global::<Settings>().on_remove_folder({
        let controller = Rc::clone(controller);
        move |id| controller.remove_folder(&id)
    });

    window.global::<Settings>().on_synchronise_folder({
        let controller = Rc::clone(controller);
        move |id| controller.synchronise_folder(&id)
    });

    window.global::<Settings>().on_restore_track({
        let controller = Rc::clone(controller);
        move |id| controller.restore_track(&id)
    });

    window.global::<Library>().on_set_order({
        let controller = Rc::clone(controller);
        move |order| controller.set_order(&order)
    });

    window.on_start_here({
        let controller = Rc::clone(controller);
        move |name, folder, find| controller.start_here(&name, folder, find)
    });

    window.global::<Settings>().on_forget_gone({
        let controller = Rc::clone(controller);
        move || controller.forget_gone()
    });

    window.global::<Settings>().on_rename_profile({
        let controller = Rc::clone(controller);
        move |name| controller.rename_profile(&name)
    });

    window.global::<Settings>().on_save_backup({
        let controller = Rc::clone(controller);
        move || controller.save_backup()
    });

    window.global::<Settings>().on_restore_backup({
        let controller = Rc::clone(controller);
        move || controller.restore_backup()
    });

    window.global::<Library>().on_pick({
        let controller = Rc::clone(controller);
        move |row, add, range| controller.pick(PickedIn::Library, row, add, range)
    });
    window.global::<Library>().on_queue_picked({
        let controller = Rc::clone(controller);
        move || controller.queue_picked(PickedIn::Library)
    });
    window.global::<Library>().on_remove_picked({
        let controller = Rc::clone(controller);
        move || controller.remove_picked(PickedIn::Library)
    });
    window.global::<Library>().on_unpick({
        let controller = Rc::clone(controller);
        move || controller.unpick(PickedIn::Library)
    });
    window.global::<Playlists>().on_pick_in_open({
        let controller = Rc::clone(controller);
        move |row, add, range| controller.pick(PickedIn::Playlist, row, add, range)
    });
    window.global::<Playlists>().on_queue_open_picked({
        let controller = Rc::clone(controller);
        move || controller.queue_picked(PickedIn::Playlist)
    });
    window.global::<Playlists>().on_remove_open_picked({
        let controller = Rc::clone(controller);
        move || controller.remove_picked(PickedIn::Playlist)
    });
    window.global::<Playlists>().on_unpick_open({
        let controller = Rc::clone(controller);
        move || controller.unpick(PickedIn::Playlist)
    });

    window.global::<Shelves>().on_open_artist({
        let controller = Rc::clone(controller);
        move |id| controller.open_artist(&id)
    });
    window.global::<Shelves>().on_play_artist({
        let controller = Rc::clone(controller);
        move |id| controller.play_artist(&id)
    });
    window.global::<Shelves>().on_play_open({
        let controller = Rc::clone(controller);
        move |id| controller.play_open(Some(&id))
    });
    window.global::<Shelves>().on_play_open_all({
        let controller = Rc::clone(controller);
        move || controller.play_open(None)
    });
    window.global::<Shelves>().on_shuffle_artist({
        let controller = Rc::clone(controller);
        move |id| controller.shuffle_artist(&id)
    });
    window.global::<Shelves>().on_shuffle_open({
        let controller = Rc::clone(controller);
        move || controller.shuffle_open()
    });

    window.global::<Playlists>().on_move_in_playlist({
        let controller = Rc::clone(controller);
        move |from, to| controller.move_in_playlist(from, to)
    });

    window.global::<Queue>().on_reorder({
        let controller = Rc::clone(controller);
        move |from, to| controller.reorder_queue(from, to)
    });

    window.global::<Playlists>().on_export_playlist({
        let controller = Rc::clone(controller);
        move |id| controller.export_playlist(&id)
    });

    window.global::<Playlists>().on_import_playlist({
        let controller = Rc::clone(controller);
        move || controller.import_playlist()
    });

    window.global::<Settings>().on_scan_now({
        let controller = Rc::clone(controller);
        move || controller.scan_now()
    });

    window.global::<Equaliser>().on_reset({
        let controller = Rc::clone(controller);
        move || controller.reset_eq()
    });
}

/// The three buttons and the drag, which the system frame used to provide.
///
/// None of this reaches the application layer: moving a window is not a use
/// case, it is the window.
fn wire_window_controls(window: &AppWindow, controller: &Rc<Controller>) {
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
                window.set_window_maximized(maximized);
            }
        }
    });

    window.on_close_window({
        let handle = window.as_weak();
        let controller = Rc::clone(controller);
        move || {
            controller.keep_placement();
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
                window.set_window_maximized(false);
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

/// A place kept on a screen that has since been unplugged would open the
/// window where nobody can see it. Checked once the window exists, which is
/// a little after the loop starts - until then this looks again shortly. If
/// no screen holds the window's middle, it goes to the middle of the main one.
fn keep_on_screen(handle: slint::Weak<AppWindow>) {
    Timer::single_shot(Duration::from_millis(50), move || {
        let Some(window) = handle.upgrade() else {
            return;
        };
        let checked = window.window().with_winit_window(|frame| {
            let Ok(corner) = frame.outer_position() else {
                return;
            };
            let size = frame.outer_size();
            let middle = (
                corner.x + i32::try_from(size.width / 2).unwrap_or(0),
                corner.y + i32::try_from(size.height / 2).unwrap_or(0),
            );
            let seen = frame.available_monitors().any(|screen| {
                let at = screen.position();
                let extent = screen.size();
                middle.0 >= at.x
                    && middle.1 >= at.y
                    && i64::from(middle.0) < i64::from(at.x) + i64::from(extent.width)
                    && i64::from(middle.1) < i64::from(at.y) + i64::from(extent.height)
            });
            if seen {
                return;
            }
            if let Some(screen) = frame
                .primary_monitor()
                .or_else(|| frame.available_monitors().next())
            {
                let at = screen.position();
                let extent = screen.size();
                frame.set_outer_position(i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                    at.x + (i64::from(extent.width) - i64::from(size.width)).max(0) as i32 / 2,
                    at.y + (i64::from(extent.height) - i64::from(size.height)).max(0) as i32 / 2,
                ));
            }
        });
        if checked.is_none() {
            keep_on_screen(handle);
        }
    });
}
