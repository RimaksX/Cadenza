//! What happens when the listener does something.
//!
//! The controller owns the direction of travel: properties down into the
//! window, commands up into the application layer. It holds no rules — every
//! method here is a translation and a call (PROJECT_MASTER 4.3).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use cadenza_core::domain::eq::{EqBand, EqMode};
use cadenza_core::domain::ids::{
    EqPresetId, ImportReviewId, MediaFileId, MoodId, PlaylistId, ProfileId,
};
use cadenza_core::domain::playback::PlaybackState;
use cadenza_core::domain::policies::eq_policy::{MAX_BAND_HZ, MAX_BAND_Q, MIN_BAND_HZ, MIN_BAND_Q};
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::queue::RepeatMode;
use cadenza_core::domain::radio::{MIN_BATCH_SIZE, RadioFeedback};
use cadenza_core::domain::review::ReviewResolution;
use cadenza_core::domain::settings::{CrossfadeDuration, InterfaceScale};
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;
use cadenza_core::domain::value_objects::{DurationMs, GainDb, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};

use crate::view_models::{
    self, eq_vm, library_vm, player_vm, playlist_vm, profile_vm, radio_vm, review_vm, stats_vm,
};
use crate::{
    AppWindow, EqBandData, FolderRowData, MoodRowData, ProfileRowData, ReviewRowData, Theme,
    TopTrackData, UiServices,
};

/// How many bars the player bar draws.
///
/// The engine folds the spectrum into exactly as many as it is asked for; this
/// is what the square in the player bar has room to separate.
const SPECTRUM_BARS: usize = 8;

/// How many heights a bar can take.
///
/// The square gives it forty-six pixels, so anything finer is a difference the
/// window would round away anyway — and every write is a repaint.
const BAR_STEPS: f32 = 46.0;

/// What to do when there is no profile to be a library for.
const NO_PROFILE_HINT: &str =
    "no profile yet — run:  cadenza create <your name>\nthen restart Cadenza";

/// What to do when there is a profile but nothing in it.
const NO_TRACKS_HINT: &str =
    "add a folder and scan it:\ncadenza add-folder <path> -r\ncadenza scan";

/// What to do when nothing is waiting to play.
const NO_QUEUE_HINT: &str =
    "queue a track to choose what follows\nor the library plays on by itself";

/// What to do when there are no playlists.
const NO_PLAYLISTS_HINT: &str = "press + NEW PLAYLIST to start one";

/// What to do when a search matches nothing.
const NO_MATCH_HINT: &str = "no track here answers to that";

/// What to do when a playlist has nothing in it.
const EMPTY_PLAYLIST_HINT: &str = "add tracks from the library\nwith the ··· at the end of a row";

/// Holds the services and pushes state into the window.
pub struct Controller {
    services: UiServices,
    window: Weak<AppWindow>,
    /// The active profile, kept because the theme belongs to it.
    profile: RefCell<Option<Profile>>,
    /// What the library is being filtered by, if anything.
    query: RefCell<String>,
    /// Whose cover the player bar is showing, so it is read from disk when the
    /// track changes rather than four times a second.
    showing_cover: RefCell<String>,
    /// The playlist whose page is open, if one is.
    ///
    /// Held because playing a track from a playlist has to say which playlist:
    /// the queue's entries carry it, and that is what makes the rest of the
    /// list follow rather than the rest of the library.
    open_playlist: RefCell<Option<PlaylistId>>,
    /// The bands the equaliser screen is showing.
    ///
    /// Held rather than rebuilt. Handing the window a *new* model makes Slint
    /// destroy and recreate every element the repeater built from it — and one
    /// of those is the touch area holding the pointer during a drag. That is
    /// why a fader could only ever be clicked: the first movement threw away
    /// the target that was following it.
    eq_bands: Rc<VecModel<EqBandData>>,
    /// What is being heard, as the player bar draws it.
    ///
    /// Kept alive for the same reason the equaliser's bands are: handing the
    /// window a new model thirty times a second would rebuild eight elements
    /// thirty times a second.
    spectrum: Rc<VecModel<f32>>,
    /// Which bell the equaliser's numbers are about.
    ///
    /// Interface state and nothing else: which band is being looked at changes
    /// nothing about the sound, so nothing outside the window needs telling.
    selected_band: Cell<usize>,
    /// Whether the tap is on, so it is only switched when it changes.
    visualising: Cell<bool>,
    /// The heights the window is already showing, to the pixel.
    shown_bars: RefCell<[f32; SPECTRUM_BARS]>,
}

impl Controller {
    /// Binds the controller to a window that has not been shown yet.
    pub fn new(services: UiServices, window: Weak<AppWindow>) -> Self {
        let profile = RefCell::new(services.profile.clone());
        Self {
            services,
            window,
            profile,
            query: RefCell::new(String::new()),
            showing_cover: RefCell::new(String::new()),
            open_playlist: RefCell::new(None),
            eq_bands: Rc::new(VecModel::default()),
            spectrum: Rc::new(VecModel::from(vec![0.0; SPECTRUM_BARS])),
            selected_band: Cell::new(0),
            visualising: Cell::new(false),
            shown_bars: RefCell::new([0.0; SPECTRUM_BARS]),
        }
    }

    /// Fills every property from scratch.
    pub fn refresh_all(&self) {
        self.refresh_profile();
        // After the profile: which size is chosen belongs to whoever is
        // listening.
        self.refresh_interface_scale();
        self.refresh_library();
        self.refresh_queue();
        self.refresh_playlists();
        self.refresh_eq();
        self.refresh_settings();
        self.refresh_radio();
        self.refresh_listening();
        self.refresh_reviews();
        self.refresh_player();

        if let Some(window) = self.window.upgrade() {
            window.set_spectrum(ModelRc::from(Rc::clone(&self.spectrum)));
        }
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

        let query = self.query.borrow().clone();
        let shown = library_vm::matching(&summaries, &query);

        window.set_library_summary(library_vm::found_line(&shown, &query, summaries.len()).into());
        window.set_empty_hint(if query.is_empty() {
            NO_TRACKS_HINT.into()
        } else {
            NO_MATCH_HINT.into()
        });
        window.set_tracks(ModelRc::new(VecModel::from(library_vm::rows(&shown))));
    }

    /// Filters the library by what has been typed into its search field.
    ///
    /// In Rust rather than in the markup: what counts as a match is a decision,
    /// and decisions made here can be tested without a window.
    pub fn search(&self, query: &str) {
        *self.query.borrow_mut() = query.to_owned();
        self.refresh_library();
    }

    /// Empties the queue, leaving what is playing where it is.
    pub fn clear_queue(&self) {
        self.run(|| self.services.queue.clear());
        self.refresh_queue();
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

        self.refresh_cover(false);

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
        window.set_playlists_summary(playlist_vm::summary_line(&summaries).into());
        window.set_playlists(ModelRc::new(VecModel::from(playlist_vm::cards(&summaries))));
        window.set_playlist_options(ModelRc::new(VecModel::from(playlist_vm::options(
            &summaries,
        ))));
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
        if let Some(window) = self.window.upgrade() {
            window.set_open_playlist_id(id.into());
        }
        self.refresh_open_playlist();
    }

    /// Makes a playlist and shows it in the index.
    pub fn create_playlist(&self, name: &str) {
        self.run(|| self.services.playlists.create(name).map(|_| ()));
        self.refresh_playlists();
    }

    /// Starts a playlist at its first track, without opening its page.
    pub fn play_playlist(&self, id: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            let tracks = self.services.playlists.tracks_of(playlist_id)?;
            let first = tracks
                .first()
                .ok_or_else(|| CoreError::invalid("playlist", "has nothing to play"))?;

            self.services
                .queue
                .play_playlist(playlist_id, &tracks, first.media_file_id)
        });
        self.refresh_queue();
    }

    /// Renames one.
    pub fn rename_playlist(&self, id: &str, name: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            self.services
                .playlists
                .rename(playlist_id, name)
                .map(|_| ())
        });
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Deletes one. Its tracks stay in the library.
    pub fn delete_playlist(&self, id: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            self.services.playlists.delete(playlist_id)
        });

        // The page for a deleted playlist has nothing to show, so the index is
        // where the listener goes back to — and the markup has already taken
        // them there, because the tile they used is on it.
        if *self.open_playlist.borrow() == PlaylistId::parse(id).ok() {
            *self.open_playlist.borrow_mut() = None;
        }
        self.refresh_playlists();
    }

    /// Puts a track at the end of a playlist.
    pub fn add_to_playlist(&self, track: &str, playlist: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(playlist)?;
            let media_file_id = MediaFileId::parse(track)?;
            self.services
                .playlists
                .add_track(playlist_id, media_file_id)
        });
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Makes a playlist and puts a track in it, which is one act rather than
    /// two: nobody names a list and then wonders why it is empty.
    pub fn create_playlist_with(&self, track: &str, name: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(track)?;
            let playlist = self.services.playlists.create(name)?;
            self.services
                .playlists
                .add_track(playlist.id, media_file_id)
        });
        self.refresh_playlists();
    }

    /// Takes the entry at a position out of the open playlist.
    pub fn remove_from_playlist(&self, position: i32) {
        let (Some(playlist_id), Ok(position)) =
            (*self.open_playlist.borrow(), usize::try_from(position))
        else {
            return;
        };

        self.run(|| self.services.playlists.remove_at(playlist_id, position));
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Takes a track out of this profile's library.
    ///
    /// A tombstone rather than a delete: the file stays on disk and every other
    /// profile keeps its own copy of the row (PROJECT_MASTER 2.1).
    pub fn remove_from_library(&self, track: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(track)?;
            self.services.library.remove_track(media_file_id)
        });

        // It may have been in lists and in the queue, and both of those show
        // titles they can no longer resolve.
        self.refresh_library();
        self.refresh_playlists();
        self.refresh_open_playlist();
        self.refresh_queue();
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
        self.refresh_radio();
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
        // Choosing a track ends the station, so the radio screen is now saying
        // something that is no longer true — that a mood is playing, and that
        // there is something to give a verdict about.
        self.refresh_radio();
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

    /// Jumps to a waiting track, by its place in the queue's own listing.
    ///
    /// Not [`Self::play`]: that starts a track *and builds a queue behind it*
    /// from the library, which is the last thing a click inside the queue
    /// should do.
    pub fn play_queued(&self, position: i32) {
        let Ok(position) = usize::try_from(position) else {
            return;
        };
        self.run(|| self.services.queue.play_at(position));
        self.refresh_queue();
    }

    /// Takes a track out of the queue, by its place in the queue's own listing.
    pub fn remove_from_queue(&self, position: i32) {
        let Ok(position) = usize::try_from(position) else {
            return;
        };
        self.run(|| self.services.queue.remove_at(position));
        self.refresh_queue();
    }

    /// Moves to the next track.
    pub fn next(&self) {
        // Pressing next during a station is a verdict on what is playing —
        // the weakest kind, but the one 10.5 asks radio to learn from. Recorded
        // before the track changes, because after it there is nothing to point
        // at. A track that ran out on its own is not a skip and does not come
        // through here.
        if self.services.radio.session().is_some()
            && let Some(track) = self.services.playback.view().track
        {
            self.run(|| {
                self.services
                    .radio
                    .feedback(track.media_file_id, RadioFeedback::Skip)
            });
        }

        self.run(|| self.services.queue.next());
        self.refresh_queue();
        self.refresh_radio();
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

    /// Everything the equaliser screen draws, presets included.
    pub fn refresh_eq(&self) {
        self.refresh_eq_controls();

        let (Some(window), Ok(setting), Ok(presets)) = (
            self.window.upgrade(),
            self.services.eq.current(),
            self.services.eq.list(),
        ) else {
            return;
        };

        let rows = eq_vm::presets(&presets, &setting);
        window.set_eq_presets(ModelRc::new(VecModel::from(rows)));
    }

    /// The controls alone: the dial, the curve, and what they read out.
    ///
    /// Separate from the presets because this runs on every step of a drag, and
    /// listing nine presets to find out that none of them is lit is a database
    /// query per pointer event.
    fn refresh_eq_controls(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Ok(setting) = self.services.eq.current() else {
            return;
        };

        let selected = self
            .selected_band
            .get()
            .min(setting.advanced.len().saturating_sub(1));
        window.set_eq_advanced(setting.mode == EqMode::Advanced);
        window.set_eq_bass(setting.simple.bass.as_db());
        window.set_eq_mid(setting.simple.mid.as_db());
        window.set_eq_treble(setting.simple.treble.as_db());
        window.set_eq_summary(eq_vm::summary_line(&setting).into());
        window.set_eq_selected(selected as i32);

        if let Some(band) = setting.advanced.get(selected) {
            window.set_eq_selected_frequency(eq_vm::hertz(band.frequency_hz()).into());
            window.set_eq_selected_q(format!("{:.1}", band.q()).into());
            window.set_eq_selected_gain(eq_vm::decibels(band.gain().as_db()).into());
        }

        // Written into the model that is already there, row by row. The first
        // pass fills it; from then on the elements the window built stay put,
        // which is what lets a fader be dragged rather than only clicked.
        let bands = eq_vm::bands(&setting, selected);
        if self.eq_bands.row_count() == bands.len() {
            for (index, band) in bands.into_iter().enumerate() {
                self.eq_bands.set_row_data(index, band);
            }
        } else {
            while self.eq_bands.row_count() > 0 {
                self.eq_bands.remove(0);
            }
            for band in bands {
                self.eq_bands.push(band);
            }
            window.set_eq_bands(ModelRc::from(Rc::clone(&self.eq_bands)));
        }
    }

    /// Switches between the three controls and the eight bells.
    pub fn set_eq_mode(&self, advanced: bool) {
        let mode = if advanced {
            EqMode::Advanced
        } else {
            EqMode::Simple
        };
        self.run(|| self.services.eq.set_mode(mode));
        self.refresh_eq();
    }

    /// Moves one of the three tone controls.
    ///
    /// The sound follows the hand and the database waits: a control being
    /// dragged reports every step of the way, and twenty-eight rows written per
    /// step is a database asked to keep up with a wrist.
    pub fn set_eq_simple(&self, which: i32, decibels: f32) {
        self.run(|| {
            let mut setting = self.services.eq.current()?;
            let gain = GainDb::clamped(decibels);
            match which {
                0 => setting.simple.bass = gain,
                1 => setting.simple.mid = gain,
                _ => setting.simple.treble = gain,
            }
            setting.mode = EqMode::Simple;
            self.services.eq.preview(setting)
        });
        self.refresh_eq_controls();
    }

    /// Sets one band's gain from where its fader was left.
    pub fn move_eq_band(&self, index: i32, y: f32) {
        let index = index.max(0) as usize;
        self.selected_band.set(index);

        self.run(|| {
            let mut setting = self.services.eq.current()?;
            let band = *setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            setting.advanced[index] = band.with_gain(GainDb::clamped(eq_vm::y_to_gain(y)));
            setting.mode = EqMode::Advanced;
            self.services.eq.preview(setting)
        });
        self.refresh_eq_controls();
    }

    /// The hand let go of a control: write down what it left behind.
    pub fn settle_eq(&self) {
        self.run(|| self.services.eq.commit());
        self.refresh_eq();
    }

    /// Says which bell the numbers under the curve are about.
    pub fn select_eq_band(&self, index: i32) {
        self.selected_band.set(index.max(0) as usize);
        self.refresh_eq();
    }

    /// Moves the chosen band along the spectrum, a sixth of an octave at a time.
    ///
    /// Buttons rather than a drag: a row of faders says nothing about where a
    /// band sits, so the frequency needs a control of its own — and a step that
    /// is a fraction of an octave moves by the same *musical* amount wherever
    /// the band happens to be.
    pub fn tune_eq_band(&self, index: i32, direction: i32) {
        let index = index.max(0) as usize;

        self.run(|| {
            let setting = self.services.eq.current()?;
            let band = setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            let moved = f64::from(band.frequency_hz()) * 2.0_f64.powf(f64::from(direction) / 6.0);
            let frequency_hz = (moved.round() as u32).clamp(MIN_BAND_HZ, MAX_BAND_HZ);

            self.services
                .eq
                .set_band(index, EqBand::new(frequency_hz, band.q(), band.gain())?)
        });
        self.refresh_eq();
    }

    /// Widens or narrows a bell without moving it.
    pub fn widen_eq_band(&self, index: i32, step: f32) {
        let index = index.max(0) as usize;

        self.run(|| {
            let setting = self.services.eq.current()?;
            let band = setting
                .advanced
                .get(index)
                .ok_or_else(|| CoreError::not_found("eq band", index))?;

            // Rounded to the tenth the readout shows. Left at full precision
            // a step out and back lands on 0.99999994 rather than 1, and a
            // preset that had been chosen would stop matching itself over a
            // difference nobody can hear or see.
            let q = (((band.q() + step) * 10.0).round() / 10.0).clamp(MIN_BAND_Q, MAX_BAND_Q);

            self.services
                .eq
                .set_band(index, EqBand::new(band.frequency_hz(), q, band.gain())?)
        });
        self.refresh_eq();
    }

    /// Sets everything at once from a preset.
    pub fn pick_eq_preset(&self, id: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.apply_preset(id));
        self.refresh_eq();
    }

    /// Saves what is set now under a name of the listener's own.
    pub fn save_eq_preset(&self, name: &str) {
        self.run(|| self.services.eq.save_as(name).map(|_| ()));
        self.refresh_eq();
    }

    /// Gives one of the listener's own sounds another name.
    pub fn rename_eq_preset(&self, id: &str, name: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.rename(id, name));
        self.refresh_eq();
    }

    /// Throws one away. What is playing is unchanged.
    pub fn delete_eq_preset(&self, id: &str) {
        let Ok(id) = EqPresetId::parse(id) else {
            return;
        };
        self.run(|| self.services.eq.delete(id));
        self.refresh_eq();
    }

    /// Puts the equaliser back to doing nothing.
    pub fn reset_eq(&self) {
        self.run(|| self.services.eq.reset());
        self.refresh_eq();
    }

    /// Everything the settings screen draws.
    /// Re-reads what is waiting for a decision.
    pub fn refresh_reviews(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let cards = match self.services.library.review_cards() {
            Ok(cards) => cards,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let rows: Vec<ReviewRowData> = cards
            .iter()
            .map(|card| ReviewRowData {
                id: card.id.to_string().into(),
                path: card
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
                    .into(),
                reason: review_vm::reason(card.reason).into(),
                existing: review_vm::collides_with(card).into(),
                duplicate: card.existing.is_some(),
            })
            .collect();

        window.set_reviews_summary(review_vm::summary_line(cards.len()).into());
        window.set_reviews(ModelRc::new(VecModel::from(rows)));
    }

    /// Applies a decision and takes the row away.
    pub fn decide_review(&self, id: &str, choice: &str) {
        self.run(|| {
            let review_id = ImportReviewId::parse(id)?;
            let resolution = match choice {
                "keep" => ReviewResolution::KeepExisting,
                "replace" => ReviewResolution::RemoveExisting,
                _ => ReviewResolution::AddAnyway,
            };
            self.services.library.resolve_review(review_id, resolution)
        });

        self.refresh_reviews();
        self.after_library_change();
    }

    /// Opens the editor on a track, or closes it when the id is empty.
    ///
    /// What it is called now is read here rather than taken from the row: a
    /// listing elides long titles, and the field would then offer the listener
    /// their own title with a dash in the middle of it.
    pub fn edit_track(&self, id: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        if id.is_empty() {
            window.set_editing_id(String::new().into());
            return;
        }

        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            let track = self
                .services
                .library
                .summaries()?
                .into_iter()
                .find(|summary| summary.media_file_id == media_file_id)
                .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

            window.set_editing_title(track.title.as_str().into());
            window.set_editing_artist(track.artist.clone().unwrap_or_default().into());
            window.set_editing_album(track.album.clone().unwrap_or_default().into());
            window.set_editing_id(id.into());
            Ok(())
        });
    }

    /// Writes a correction down and closes the editor.
    pub fn save_track(&self, id: &str, title: &str, artist: &str, album: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services
                .library
                .edit_track(media_file_id, title, Some(artist), Some(album))
        });

        if let Some(window) = self.window.upgrade() {
            window.set_editing_id(String::new().into());
        }
        self.after_library_change();
    }

    /// Switches to another listener.
    ///
    /// PROJECT_MASTER 2.5 states this as three steps in order — playback stops,
    /// the outgoing profile's state is saved, the incoming one's is loaded —
    /// and the order is the whole of it: a queue reloaded before playback stops
    /// would be the new listener's queue with the old listener's track playing
    /// out of it.
    ///
    /// Sequenced here because this is where all three services meet. Each step
    /// is the service's own: stopping is playback's, saving is what the queue
    /// does after every change it makes, and loading is `reload`.
    pub fn switch_profile(&self, id: &str) {
        self.run(|| {
            let profile_id = ProfileId::parse(id)?;
            self.services.playback.stop()?;

            let profile = self.services.profiles.switch_to(profile_id)?;
            self.services.queue.reload();
            self.services.radio.stop();

            *self.profile.borrow_mut() = Some(profile);
            Ok(())
        });

        self.refresh_all();
    }

    /// Adds a listener and hands the application over to them.
    ///
    /// Switching straight away because that is what somebody who has just made
    /// one wants: a profile nobody is using is a row in a table.
    pub fn create_profile(&self, name: &str) {
        let created = self.services.profiles.create(name);
        match created {
            Ok(profile) => self.switch_profile(&profile.id.to_string()),
            Err(err) => self.report(&err),
        }
    }

    /// Re-reads the month of listening.
    pub fn refresh_listening(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let report = match self.services.stats.report() {
            Ok(report) => report,
            Err(CoreError::NoActiveProfile) => return,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        window.set_listening_kept(report.keeping);
        window.set_listening_summary(stats_vm::summary_line(&report).into());
        window.set_listening_heard(stats_vm::heard(report.summary.listened).into());
        window.set_listening_listens(report.summary.completed.to_string().into());
        window.set_listening_tracks(report.summary.tracks.to_string().into());
        window.set_listening_skip_rate(stats_vm::skip_rate(&report).into());

        let rows: Vec<TopTrackData> = report
            .top
            .iter()
            .enumerate()
            .map(|(index, (track, count))| TopTrackData {
                rank: format!("{}", index + 1).into(),
                title: track.title.as_str().into(),
                subtitle: track.artist.clone().unwrap_or_default().into(),
                plays: stats_vm::plays(*count).into(),
            })
            .collect();
        window.set_top_tracks(ModelRc::new(VecModel::from(rows)));
    }

    /// Re-reads a screen the listener has just moved to.
    ///
    /// A page is drawn from a service at the moment something asks it to be,
    /// and between one visit and the next the service may have moved on
    /// without the page being told — a station ends because a track was
    /// played, a folder is scanned, a preset is renamed. Asking on arrival
    /// costs one query on a keypress and closes the whole class rather than
    /// the one case somebody happened to notice.
    /// Asks for a picture and puts it on a track.
    ///
    /// The chooser blocks the interface thread, which is what a modal dialog
    /// does. Nothing else may call it, and nothing else does.
    pub fn choose_cover(&self, id: &str) {
        let Ok(media_file_id) = MediaFileId::parse(id) else {
            return;
        };
        self.run(|| self.services.library.choose_cover(media_file_id).map(drop));
        self.refresh_cover(true);
    }

    /// Takes a chosen cover off, leaving whatever the file itself carries.
    pub fn clear_cover(&self, id: &str) {
        let Ok(media_file_id) = MediaFileId::parse(id) else {
            return;
        };
        self.run(|| self.services.library.clear_cover(media_file_id));
        self.refresh_cover(true);
    }

    /// The same two, for a list.
    pub fn choose_playlist_cover(&self, id: &str) {
        let Ok(playlist_id) = PlaylistId::parse(id) else {
            return;
        };
        self.run(|| self.services.playlists.choose_cover(playlist_id).map(drop));
        self.refresh_playlists();
    }

    pub fn clear_playlist_cover(&self, id: &str) {
        let Ok(playlist_id) = PlaylistId::parse(id) else {
            return;
        };
        self.run(|| self.services.playlists.clear_cover(playlist_id));
        self.refresh_playlists();
    }

    /// Puts the playing track's cover in the player bar.
    ///
    /// Only when the track changed, or when somebody has just chosen one: this
    /// is asked four times a second, and reading an image off the disk that
    /// often would be paid for in frames for a picture that changes when the
    /// music does.
    fn refresh_cover(&self, force: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let playing = window.get_playing_id().to_string();
        if !force && *self.showing_cover.borrow() == playing {
            return;
        }
        *self.showing_cover.borrow_mut() = playing.clone();

        let cover = MediaFileId::parse(&playing)
            .ok()
            .and_then(|id| self.services.library.cover_for(id).ok().flatten())
            .and_then(|path| slint::Image::load_from_path(&path).ok())
            .unwrap_or_default();

        window.set_now_cover(cover);
    }

    /// Draws the interface at the size this listener chose.
    ///
    /// The scale goes into the *lengths*, through `Theme.scale`, rather than
    /// into the renderer. A renderer told to draw at 1.25 puts every hairline
    /// on a pixel and a quarter and every stem between two columns, which is
    /// what a magnifying glass looks like; the lengths are rounded back onto
    /// whole pixels before anything is drawn (`MASTER_ISSUES` 65).
    ///
    /// Which also makes it live. The window is not remade, it is re-measured.
    pub fn refresh_interface_scale(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let scale = self.chosen_scale();
        window.set_ui_scale(i32::from(scale.percent()));
        window.global::<Theme>().set_scale(scale.factor());
    }

    /// How large this listener has asked for the interface to be drawn.
    fn chosen_scale(&self) -> InterfaceScale {
        self.profile
            .borrow()
            .as_ref()
            .map(|profile| profile.id)
            .and_then(|id| self.services.profiles.interface_scale(id).ok())
            .unwrap_or_default()
    }

    /// Chooses how large the interface is drawn, and draws it that way now.
    ///
    /// The window is resized by the same ratio, because the interface asking
    /// for a quarter more room does not by itself give it any: the lengths grow
    /// and the frame around them does not, so the far side of every page ends
    /// up past the edge — and on the way back down the window keeps the height
    /// it was stretched to (`MASTER_ISSUES` 66).
    pub fn set_interface_scale(&self, percent: i32) {
        let Some(profile) = self.profile.borrow().as_ref().map(|profile| profile.id) else {
            return;
        };

        let before = self.chosen_scale();
        self.run(|| {
            let scale = InterfaceScale::new(u16::try_from(percent).unwrap_or_default())?;
            self.services.profiles.set_interface_scale(profile, scale)
        });
        self.refresh_interface_scale();

        let after = self.chosen_scale();
        if after == before {
            return;
        }

        if let Some(window) = self.window.upgrade() {
            let ratio = after.factor() / before.factor();
            let size = window.window().size();
            window.window().set_size(slint::PhysicalSize::new(
                (size.width as f32 * ratio).round() as u32,
                (size.height as f32 * ratio).round() as u32,
            ));
        }
    }

    /// Whether something from outside is being held over the window.
    ///
    /// Written only when it changes: this is asked twenty times a second, and
    /// every write to a property is a repaint of whatever reads it.
    pub fn carrying_files(&self, carrying: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if window.get_dropping() != carrying {
            window.set_dropping(carrying);
        }
    }

    /// Takes in what was let go of over the window.
    ///
    /// On a thread of its own. A folder dropped in can be five thousand files,
    /// and hashing them on the event loop would freeze the interface — the one
    /// that is meanwhile drawing a progress line for the music still playing.
    /// What comes back is a sentence in the player bar; the list itself
    /// refreshes the way any other change behind the listener's back does.
    pub fn accept_drop(&self, paths: Vec<PathBuf>) {
        let library = Arc::clone(&self.services.library);
        let window = self.window.clone();

        std::thread::spawn(move || {
            let said = match library.accept_drop(&paths) {
                Ok(report) => library_vm::taken_in(&report),
                Err(err) => err.to_string(),
            };

            // Back on the event loop to say so: a window may only be touched
            // from the thread that runs it.
            let _ = window.upgrade_in_event_loop(move |window| window.set_message(said.into()));
        });
    }

    /// Re-reads what a change nobody in this window made could have altered.
    ///
    /// Three readings rather than all nine: a file appearing or vanishing moves
    /// the library, can take a track out of the queue, and can put a duplicate
    /// in front of the listener. It cannot change the equaliser, the theme or a
    /// month of listening, and re-reading those four times a second through a
    /// long import would be paid for in frames.
    pub fn refresh_after_change(&self) {
        self.refresh_library();
        self.refresh_queue();
        self.refresh_reviews();
    }

    pub fn showing(&self, section: &str) {
        match section {
            "listening" => self.refresh_listening(),
            "reviews" => self.refresh_reviews(),
            "radio" => self.refresh_radio(),
            "settings" => self.refresh_settings(),
            "queue" => self.refresh_queue(),
            "playlists" => self.refresh_playlists(),
            "equaliser" => self.refresh_eq(),
            "library" => self.refresh_library(),
            _ => {}
        }
    }

    /// Re-reads who is listening and who else could be.
    fn refresh_profiles_list(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Ok(profiles) = self.services.profiles.list() else {
            return;
        };

        let active = self.profile.borrow().as_ref().map(|profile| profile.id);
        let rows: Vec<ProfileRowData> = profiles
            .iter()
            .map(|profile| ProfileRowData {
                id: profile.id.to_string().into(),
                name: profile.name.as_str().into(),
                note: profile_vm::note(profile).into(),
                active: Some(profile.id) == active,
            })
            .collect();
        window.set_profiles(ModelRc::new(VecModel::from(rows)));
    }

    /// Re-reads the moods and what the station is doing.
    pub fn refresh_radio(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let moods = match self.services.radio.moods() {
            Ok(moods) => moods,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let rows: Vec<MoodRowData> = moods
            .iter()
            .map(|mood| MoodRowData {
                id: mood.id.to_string().into(),
                name: mood.name.as_str().into(),
                asks: radio_vm::asks_for(&mood.rules).into(),
            })
            .collect();
        window.set_moods(ModelRc::new(VecModel::from(rows)));

        let playing = self.services.radio.mood();
        window.set_radio_mood(
            playing
                .as_ref()
                .map_or_else(String::new, |mood| mood.name.clone())
                .into(),
        );
        window.set_radio_summary(
            radio_vm::summary_line(playing.as_ref(), self.services.queue.view().pending).into(),
        );
    }

    /// Starts a station in the chosen mood, seeded by what is playing.
    pub fn start_radio(&self, id: &str) {
        self.run(|| {
            let mood_id = MoodId::parse(id)?;
            let seed = self
                .services
                .playback
                .view()
                .track
                .map(|track| track.media_file_id);

            let session = self.services.radio.start(mood_id, seed)?;
            let batch = self.services.radio.next_batch(MIN_BATCH_SIZE)?;
            self.services.queue.play_radio(session.id, &batch)
        });

        self.refresh_radio();
        self.refresh_queue();
        self.refresh_player();
    }

    /// Tells the station what the listener thinks of what is playing.
    pub fn judge_radio(&self, like: bool) {
        let Some(track) = self.services.playback.view().track else {
            return;
        };

        let verdict = if like {
            RadioFeedback::Like
        } else {
            RadioFeedback::Dislike
        };
        self.run(|| self.services.radio.feedback(track.media_file_id, verdict));
    }

    pub fn refresh_settings(&self) {
        self.refresh_profiles_list();

        let Some(window) = self.window.upgrade() else {
            return;
        };

        if let Some(profile) = self.profile.borrow().as_ref() {
            window.set_dark(profile.theme == ThemeMode::Dark);
            window.set_history_on(profile.history_enabled);
        }

        if let Ok(settings) = self.services.playback.settings() {
            window.set_crossfade_on(settings.crossfade_enabled);
            window.set_crossfade_seconds(
                (settings.crossfade.as_duration().as_millis() / 1_000) as i32,
            );
        }

        window.set_suggested_folder(
            self.services
                .library
                .suggested_folder()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
                .into(),
        );

        let Ok(folders) = self.services.library.folders() else {
            return;
        };
        let rows: Vec<FolderRowData> = folders
            .iter()
            .map(|folder| FolderRowData {
                id: folder.id.to_string().into(),
                path: folder.path.display().to_string().into(),
                note: if folder.include_subfolders {
                    "WITH SUBFOLDERS".into()
                } else {
                    "THIS FOLDER ONLY".into()
                },
            })
            .collect();
        window.set_folders(ModelRc::new(VecModel::from(rows)));

        let tracks = self
            .services
            .library
            .tracks()
            .map(|all| all.len())
            .unwrap_or(0);
        window.set_settings_summary(
            format!(
                "{} {} · {tracks} {}",
                folders.len(),
                if folders.len() == 1 {
                    "folder"
                } else {
                    "folders"
                },
                if tracks == 1 { "track" } else { "tracks" }
            )
            .into(),
        );
    }

    /// Which way round the ink and the paper go.
    pub fn set_theme(&self, dark: bool) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        let mode = if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };

        self.run(|| {
            let updated = self.services.profiles.set_theme(profile.id, mode)?;
            *self.profile.borrow_mut() = Some(updated);
            Ok(())
        });

        if let Some(window) = self.window.upgrade() {
            window.global::<Theme>().set_dark(dark);
        }
        self.refresh_settings();
    }

    /// Whether ordinary tracks fade into each other, and over how long.
    pub fn set_crossfade(&self, enabled: bool, seconds: i32) {
        self.run(|| {
            let duration = CrossfadeDuration::new(DurationMs::from_secs(seconds.max(0) as u64))?;
            self.services.playback.set_crossfade(enabled, duration)
        });
        self.refresh_settings();
    }

    /// Whether what was played is written down.
    pub fn set_history(&self, keep: bool) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        self.run(|| {
            let updated = self
                .services
                .profiles
                .set_history_enabled(profile.id, keep)?;
            *self.profile.borrow_mut() = Some(updated);
            Ok(())
        });
        self.refresh_settings();
    }

    /// Asks for a folder and takes in what is in it.
    pub fn add_folder(&self) {
        self.run(|| self.services.library.choose_folder().map(|_| ()));
        self.after_library_change();
    }

    /// Makes the folder this machine suggests and watches it.
    pub fn use_suggested_folder(&self) {
        self.run(|| self.services.library.use_suggested_folder().map(|_| ()));
        self.after_library_change();
    }

    /// Stops watching one. What was imported from it stays.
    pub fn remove_folder(&self, id: &str) {
        self.run(|| {
            let folders = self.services.library.folders()?;
            let folder = folders
                .iter()
                .find(|folder| folder.id.to_string() == id)
                .ok_or_else(|| CoreError::not_found("library folder", id))?;
            self.services.library.remove_folder(folder)
        });
        self.after_library_change();
    }

    /// Looks again at every folder, and says what it found.
    ///
    /// A scan that reports nothing looks the same whether it found nothing or
    /// was never run, which is exactly the doubt somebody presses it to settle.
    pub fn scan_now(&self) {
        let said = match self.services.library.scan_all() {
            Ok(report) => library_vm::taken_in(&report),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        if let Some(window) = self.window.upgrade() {
            window.set_message(said.into());
        }
        self.after_library_change();
    }

    /// The library moved, so everything that lists it has to look again.
    fn after_library_change(&self) {
        self.refresh_library();
        self.refresh_reviews();
        self.refresh_settings();
    }

    /// Reads what is being heard and hands it to the player bar.
    ///
    /// Called on its own timer rather than the transport's: section 2.9 caps
    /// this at thirty a second, and the transport is happy at four. Nothing is
    /// read while nothing is playing — and nothing is copied out of the audio
    /// callback either, because the tap is turned off with it.
    pub fn refresh_spectrum(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        // Playing is not enough. PROJECT_MASTER 2.9 asks for the work to stop
        // when the window is away as well, and a minimised window is the case
        // where every frame drawn is a frame nobody can see — the tap, the
        // transform and the repaint all paid for an audience of none.
        let playing = self.services.playback.view().state == PlaybackState::Playing;
        let watching = playing && !window.window().is_minimized();

        if watching != self.visualising.get() {
            self.visualising.set(watching);
            self.services.playback.set_visualising(watching);
            if !watching {
                window.set_spectrum(ModelRc::from(Rc::clone(&self.spectrum)));
            }
        }
        if !watching {
            return;
        }

        let mut bars = [0.0_f32; SPECTRUM_BARS];
        if !self.services.playback.spectrum(&mut bars) {
            return;
        }

        // Rounded to the pixel it will be drawn at, and written only where that
        // pixel moved. Touching the model is what makes the window repaint, and
        // a repaint is the whole cost of this: the engine and the transform
        // together are a third of a per cent of one core, and drawing thirty
        // frames a second is twenty-six times that. A bar that has not visibly
        // changed is a frame nobody needs.
        let mut shown = self.shown_bars.borrow_mut();
        for (index, height) in bars.into_iter().enumerate() {
            let stepped = (height * BAR_STEPS).round() / BAR_STEPS;
            if (stepped - shown[index]).abs() < f32::EPSILON {
                continue;
            }
            shown[index] = stepped;
            self.spectrum.set_row_data(index, stepped);
        }
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
