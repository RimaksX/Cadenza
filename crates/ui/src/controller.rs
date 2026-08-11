//! What happens when the listener does something.
//!
//! The controller owns the direction of travel: properties down into the
//! window, commands up into the application layer. It holds no rules — every
//! method here is a translation and a call (PROJECT_MASTER 4.3).

use std::cell::RefCell;

use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;
use cadenza_core::domain::value_objects::{PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};

use crate::view_models::{self, library_vm, player_vm};
use crate::{AppWindow, Theme, UiServices};

/// What to do when there is no profile to be a library for.
const NO_PROFILE_HINT: &str =
    "no profile yet — run:  cadenza create <your name>\nthen restart Cadenza";

/// What to do when there is a profile but nothing in it.
const NO_TRACKS_HINT: &str =
    "add a folder and scan it:\ncadenza add-folder <path> -r\ncadenza scan";

/// Holds the services and pushes state into the window.
pub struct Controller {
    services: UiServices,
    window: Weak<AppWindow>,
    /// The active profile, kept because the theme belongs to it.
    profile: RefCell<Option<Profile>>,
}

impl Controller {
    /// Binds the controller to a window that has not been shown yet.
    pub fn new(services: UiServices, window: Weak<AppWindow>) -> Self {
        let profile = RefCell::new(services.profile.clone());
        Self {
            services,
            window,
            profile,
        }
    }

    /// Fills every property from scratch.
    pub fn refresh_all(&self) {
        self.refresh_profile();
        self.refresh_library();
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
    }

    /// Starts the track a row identifies.
    pub fn play(&self, id: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services.playback.play_track(media_file_id)
        });
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
