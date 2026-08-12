//! What happens when the listener does something.
//!
//! The controller owns the direction of travel: properties down into the
//! window, commands up into the application layer. It holds no rules — every
//! method here is a translation and a call (PROJECT_MASTER 4.3).

use std::cell::RefCell;

use cadenza_core::domain::ids::{MediaFileId, PlaylistId};
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::queue::RepeatMode;
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;
use cadenza_core::domain::value_objects::{PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};

use crate::view_models::{self, library_vm, player_vm, playlist_vm};
use crate::{AppWindow, Theme, UiServices};

/// What to do when there is no profile to be a library for.
const NO_PROFILE_HINT: &str =
    "no profile yet — run:  cadenza create <your name>\nthen restart Cadenza";

/// What to do when there is a profile but nothing in it.
const NO_TRACKS_HINT: &str =
    "add a folder and scan it:\ncadenza add-folder <path> -r\ncadenza scan";

/// What to do when nothing is waiting to play.
const NO_QUEUE_HINT: &str = "play something from the library\nand the rest follows it";

/// What to do when there are no playlists.
///
/// Making one is a command-line job for now, the way adding a folder is: it
/// needs a name typed into a field this interface does not have yet.
const NO_PLAYLISTS_HINT: &str =
    "make one:\ncadenza playlist new <name>\ncadenza playlist add <name> <track number>";

/// What to do when a playlist has nothing in it.
const EMPTY_PLAYLIST_HINT: &str = "add to it:\ncadenza playlist add <name> <track number>";

/// Holds the services and pushes state into the window.
pub struct Controller {
    services: UiServices,
    window: Weak<AppWindow>,
    /// The active profile, kept because the theme belongs to it.
    profile: RefCell<Option<Profile>>,
    /// The playlist whose page is open, if one is.
    ///
    /// Held because playing a track from a playlist has to say which playlist:
    /// the queue's entries carry it, and that is what makes the rest of the
    /// list follow rather than the rest of the library.
    open_playlist: RefCell<Option<PlaylistId>>,
}

impl Controller {
    /// Binds the controller to a window that has not been shown yet.
    pub fn new(services: UiServices, window: Weak<AppWindow>) -> Self {
        let profile = RefCell::new(services.profile.clone());
        Self {
            services,
            window,
            profile,
            open_playlist: RefCell::new(None),
        }
    }

    /// Fills every property from scratch.
    pub fn refresh_all(&self) {
        self.refresh_profile();
        self.refresh_library();
        self.refresh_queue();
        self.refresh_playlists();
        self.refresh_player();
    }

    /// Who is listening, and in which theme.
    fn refresh_profile(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        match self.profile.borrow().as_ref() {
            Some(profile) => {
                window.set_profile_name(profile.name.as_str().into());
                window.set_profile_initial(view_models::initial(profile.name.as_str()).into());
                window
                    .global::<Theme>()
                    .set_dark(profile.theme == ThemeMode::Dark);
            }
            None => {
                window.set_profile_name("nobody".into());
                window.set_profile_initial(String::new().into());
                window.global::<Theme>().set_dark(true);
            }
        }
    }

    /// Re-reads the library. Cheap enough to call on any change that could have
    /// touched it, and the only thing that reads the whole table.
    pub fn refresh_library(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let summaries = match self.services.library.summaries() {
            Ok(summaries) => summaries,
            // Not an error worth reporting: it is the first-run state, and the
            // empty view already says what to do about it.
            Err(CoreError::NoActiveProfile) => {
                window.set_tracks(ModelRc::new(VecModel::from(Vec::new())));
                window.set_library_summary("no profile".into());
                window.set_empty_hint(NO_PROFILE_HINT.into());
                return;
            }
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        window.set_library_summary(library_vm::summary_line(&summaries).into());
        window.set_empty_hint(NO_TRACKS_HINT.into());
        window.set_tracks(ModelRc::new(VecModel::from(library_vm::rows(&summaries))));
    }

    /// Re-reads the transport. Called on every command and on the tick.
    pub fn refresh_player(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let shown = player_vm::fields(&self.services.playback.view());

        window.set_now_title(shown.title.as_str().into());
        window.set_now_subtitle(shown.subtitle.as_str().into());
        window.set_now_initial(shown.initial.as_str().into());
        window.set_now_position(shown.position.as_str().into());
        window.set_now_duration(shown.duration.as_str().into());
        window.set_playing_id(shown.playing_id.as_str().into());
        window.set_progress(shown.progress);
        window.set_playing(shown.playing);
        window.set_loaded(shown.loaded);
        window.set_muted(shown.muted);
        window.set_volume(shown.volume);

        let queue = self.services.queue.view();
        window.set_shuffle(queue.shuffle);
        window.set_repeating(queue.repeats());
        window.set_repeat_one(queue.repeat == RepeatMode::One);
        window.set_has_next(queue.has_next);
        window.set_has_previous(queue.has_previous);
    }

    /// Re-reads what is waiting to play.
    ///
    /// Not on the tick: it reads the whole library to resolve titles, and the
    /// queue only changes when something is asked of it.
    pub fn refresh_queue(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let waiting = match self.services.queue.upcoming() {
            Ok(waiting) => waiting,
            // The same first-run state the library shows; the empty view says
            // what to do about it.
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        window.set_queue_hint(NO_QUEUE_HINT.into());
        window.set_queue_summary(library_vm::summary_line(&waiting).into());
        window.set_queue_tracks(ModelRc::new(VecModel::from(library_vm::rows(&waiting))));
    }

    /// Re-reads the index of playlists.
    pub fn refresh_playlists(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let summaries = match self.services.playlists.list() {
            Ok(summaries) => summaries,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        window.set_playlists_hint(NO_PLAYLISTS_HINT.into());
        window.set_playlists(ModelRc::new(VecModel::from(playlist_vm::rows(&summaries))));
    }

    /// Opens one playlist's page.
    pub fn open_playlist(&self, id: &str) {
        let playlist_id = match PlaylistId::parse(id) {
            Ok(playlist_id) => playlist_id,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        *self.open_playlist.borrow_mut() = Some(playlist_id);
        self.refresh_open_playlist();
    }

    /// Re-reads whatever playlist page is open.
    fn refresh_open_playlist(&self) {
        let (Some(window), Some(playlist_id)) =
            (self.window.upgrade(), *self.open_playlist.borrow())
        else {
            return;
        };

        let playlist = match self.services.playlists.get(playlist_id) {
            Ok(playlist) => playlist,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let tracks = match self.services.playlists.tracks_of(playlist_id) {
            Ok(tracks) => tracks,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        window.set_playlist_name(playlist.name.as_str().into());
        window.set_playlist_hint(EMPTY_PLAYLIST_HINT.into());
        window.set_playlist_summary(library_vm::summary_line(&tracks).into());
        window.set_playlist_tracks(ModelRc::new(VecModel::from(library_vm::rows(&tracks))));
    }

    /// Starts a track from the open playlist, with the rest of the list behind
    /// it.
    pub fn play_from_playlist(&self, id: &str) {
        let Some(playlist_id) = *self.open_playlist.borrow() else {
            return;
        };

        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            let tracks = self.services.playlists.tracks_of(playlist_id)?;
            self.services
                .queue
                .play_playlist(playlist_id, &tracks, media_file_id)
        });
        self.refresh_queue();
    }

    /// Advances when the current track has run out.
    ///
    /// Called from the tick alongside [`Self::refresh_player`], because nothing
    /// else can: the audio callback is forbidden from calling into the
    /// application layer (PROJECT_MASTER 8.2).
    pub fn poll_queue(&self) {
        match self.services.queue.poll() {
            // Only when a track actually changed, so the tick does not turn a
            // listing query into a background load.
            Ok(true) => self.refresh_queue(),
            Ok(false) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Starts the track a row identifies, with the rest of the library behind it.
    pub fn play(&self, id: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services.queue.play_from_library(media_file_id)
        });
        self.refresh_queue();
    }

    /// Puts a track at the end of the manual queue.
    ///
    /// Nothing starts playing: the point of the manual queue is that it plays
    /// after what is on now (PROJECT_MASTER 2.3).
    pub fn enqueue(&self, id: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services.queue.enqueue(media_file_id)
        });
        self.refresh_queue();
    }

    /// Moves to the next track.
    pub fn next(&self) {
        self.run(|| self.services.queue.next());
        self.refresh_queue();
    }

    /// Restarts the track, or moves back to the previous one.
    pub fn previous(&self) {
        self.run(|| self.services.queue.previous());
        self.refresh_queue();
    }

    /// Turns shuffle on or off.
    pub fn toggle_shuffle(&self) {
        self.run(|| self.services.queue.toggle_shuffle());
        self.refresh_queue();
    }

    /// Steps through the repeat modes.
    pub fn cycle_repeat(&self) {
        self.run(|| self.services.queue.cycle_repeat());
        self.refresh_queue();
    }

    /// Pauses or resumes.
    pub fn toggle_play(&self) {
        self.run(|| self.services.playback.toggle());
    }

    /// Jumps to a fraction of the track.
    ///
    /// The bar reports where it was clicked, not a time: it knows its own width
    /// and nothing about how long the track is.
    pub fn seek(&self, fraction: f32) {
        self.run(|| {
            let view = self.services.playback.view();
            let millis =
                (f64::from(fraction.clamp(0.0, 1.0)) * view.duration.as_millis() as f64) as u64;
            self.services
                .playback
                .seek(PlaybackPosition::from_millis(millis))
        });
    }

    /// Sets the output level.
    pub fn set_volume(&self, level: f32) {
        self.run(|| self.services.playback.set_volume(Volume::clamped(level)));
    }

    /// Silences output, or restores it.
    pub fn toggle_mute(&self) {
        self.run(|| self.services.playback.toggle_mute());
    }

    /// Runs a command, reports what it says, and refreshes the transport.
    fn run(&self, command: impl FnOnce() -> Result<()>) {
        match command() {
            Ok(()) => self.clear_message(),
            Err(err) => self.report(&err),
        }
        self.refresh_player();
    }

    /// Puts a failure where the listener will see it.
    ///
    /// The player bar rather than a dialog: nothing here is fatal, and a modal
    /// over a missing file would be a worse interruption than the silence
    /// already is.
    fn report(&self, err: &CoreError) {
        if let Some(window) = self.window.upgrade() {
            window.set_message(err.to_string().into());
        }
    }

    fn clear_message(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_message(String::new().into());
        }
    }
}
