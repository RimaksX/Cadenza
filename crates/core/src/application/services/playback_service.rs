//! Playing a track.
//!
//! The interface is not allowed to drive the audio engine itself
//! (PROJECT_MASTER 4.3), so this is the whole of what it may ask for: load a
//! track, start, stop, move, change the level. Everything the player bar shows
//! comes back as one [`PlayerView`].

use std::sync::{Arc, RwLock};

use crate::application::context::AppContext;
use crate::application::view_state::PlayerView;
use crate::domain::ids::{MediaFileId, PlayEventId, ProfileId};
use crate::domain::playback::{PlaybackState, TransitionProfile};
use crate::domain::policies::history_policy::{classify, should_record};
use crate::domain::ports::audio_engine::AudioEnginePort;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::repositories::{
    MediaFileRepositoryPort, PlayEventRepositoryPort, TrackRepositoryPort,
};
use crate::domain::settings::{
    CROSSFADE_ENABLED_KEY, CROSSFADE_MS_KEY, CrossfadeDuration, PRELOAD_NEXT_KEY, PlaybackSettings,
    SettingValue, VOLUME_KEY,
};
use crate::domain::stats::{PlayEvent, PlaySource};
use crate::domain::track::TrackSummary;
use crate::domain::value_objects::{DurationMs, PlaybackPosition, Timestamp, Volume};
use crate::{CoreError, Result};

/// Everything playback talks to.
pub struct PlaybackPorts {
    /// The audio output.
    pub engine: Arc<dyn AudioEnginePort>,
    /// The global catalogue, for the path of the file to play.
    pub media_files: Arc<dyn MediaFileRepositoryPort>,
    /// Per-profile library membership, for what to show while it plays.
    pub tracks: Arc<dyn TrackRepositoryPort>,
    /// Where a listen is written down, when the listener allows it
    /// (PROJECT_MASTER 2.6).
    pub history: Arc<dyn PlayEventRepositoryPort>,
}

/// A listen that has started and not yet been written down.
struct Listen {
    profile_id: ProfileId,
    media_file_id: MediaFileId,
    source: PlaySource,
    /// The track's length, copied because the row will outlive the file.
    duration: DurationMs,
    started_at: Timestamp,
}

/// Transport control and the player's view state.
pub struct PlaybackService {
    context: Arc<AppContext>,
    ports: PlaybackPorts,
    /// What is loaded. Held here rather than asked of the engine, which knows a
    /// file path and nothing about titles or libraries.
    loaded: RwLock<Option<TrackSummary>>,
    /// The level the slider shows, which is not the level being output while
    /// muted. Mute has to remember what to go back to (PROJECT_MASTER 2.3).
    volume: RwLock<Volume>,
    muted: RwLock<bool>,
    /// The listen in progress, if history is being kept.
    ///
    /// Opened when a track starts and closed when it is replaced, stopped or
    /// handed over from. Held here because this is the only place that knows
    /// when a track *became* the one playing — the queue knows what should play
    /// next, which is a different moment.
    listening: RwLock<Option<Listen>>,
    /// The active profile's playback preferences, and whose they are.
    ///
    /// Cached because the queue asks for them four times a second, and three
    /// key reads per tick is a database kept busy saying the same thing. The
    /// profile travels with them so a switch cannot be answered from the last
    /// listener's settings.
    settings: RwLock<Option<(ProfileId, PlaybackSettings)>>,
}

impl PlaybackService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: PlaybackPorts) -> Self {
        Self {
            context,
            ports,
            loaded: RwLock::new(None),
            volume: RwLock::new(Volume::default()),
            muted: RwLock::new(false),
            settings: RwLock::new(None),
            listening: RwLock::new(None),
        }
    }

    /// Opens the track that will follow, so the engine can join it on.
    ///
    /// Nothing audible happens here: the file is read and decoded ahead of the
    /// join, and whether that join is a fade or a butt splice is the transition
    /// profile's business (PROJECT_MASTER 8.4).
    pub fn preload(&self, media_file_id: MediaFileId, transition: TransitionProfile) -> Result<()> {
        let media_file = self
            .ports
            .media_files
            .get(media_file_id)?
            .ok_or_else(|| CoreError::not_found("media file", media_file_id))?;

        if !media_file.state.is_playable() {
            return Err(CoreError::Audio(format!(
                "{} cannot be queued to follow: it is {}",
                media_file.path.display(),
                media_file.state.as_str()
            )));
        }

        self.ports.engine.preload_next(&media_file.path, transition)
    }

    /// Turns the visualiser's tap on or off.
    ///
    /// The window says when it is drawing and when it is not; nothing is copied
    /// out of the audio callback in between (PROJECT_MASTER 2.9).
    pub fn set_visualising(&self, on: bool) {
        self.ports.engine.set_visualising(on);
    }

    /// The spectrum of what was last played, from zero to one.
    pub fn spectrum(&self, bars: &mut [f32]) -> bool {
        self.ports.engine.spectrum(bars)
    }

    /// Whether a following track is already open and waiting.
    pub fn armed(&self) -> bool {
        self.ports.engine.armed()
    }

    /// How many times the engine has handed over to a preloaded track by itself.
    pub fn advances(&self) -> u64 {
        self.ports.engine.advances()
    }

    /// Records that the engine has moved on to a track by itself.
    ///
    /// The counterpart to [`Self::play_track`], and deliberately not it: the
    /// audio is already playing. Loading it again would flush the ring and put
    /// a hole in the middle of the join that was the whole point.
    pub fn adopt(&self, media_file_id: MediaFileId, source: PlaySource) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let summary = self
            .ports
            .tracks
            .summary(profile_id, media_file_id)?
            .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

        // A join happens because the outgoing track ran out, so it was heard to
        // its end. Its position cannot be asked for any more — the engine is
        // already reporting the new track.
        let heard = self.listened_duration();
        self.close_listen(heard);

        self.open_listen(&summary, source);
        self.write_loaded(Some(summary));
        self.announce();
        Ok(())
    }

    /// The active profile's playback preferences.
    ///
    /// Anything never chosen comes back as its documented default, and the
    /// crossfade length is pushed to the engine as it is read — the engine has
    /// no database and no way to ask.
    pub fn settings(&self) -> Result<PlaybackSettings> {
        let profile_id = self.context.require_active_profile()?;

        if let Some((cached_for, settings)) =
            *self.settings.read().unwrap_or_else(|err| err.into_inner())
            && cached_for == profile_id
        {
            return Ok(settings);
        }

        let settings = self.read_settings(profile_id)?;
        self.ports.engine.set_crossfade(settings.crossfade)?;
        *self.settings.write().unwrap_or_else(|err| err.into_inner()) =
            Some((profile_id, settings));
        Ok(settings)
    }

    /// Turns crossfading on or off and says how long it should take.
    ///
    /// Both at once, because they are one decision: a length nobody has turned
    /// on changes nothing, and turning it on without a length is a question
    /// about what the length is.
    pub fn set_crossfade(&self, enabled: bool, duration: CrossfadeDuration) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let now = self.context.now();
        let millis = i64::try_from(duration.as_duration().as_millis())
            .map_err(|_| CoreError::invalid("crossfade", "the length does not fit a number"))?;

        self.context.settings.profile_set(
            profile_id,
            CROSSFADE_ENABLED_KEY,
            &SettingValue::Bool(enabled),
            now,
        )?;
        self.context.settings.profile_set(
            profile_id,
            CROSSFADE_MS_KEY,
            &SettingValue::Integer(millis),
            now,
        )?;

        self.ports.engine.set_crossfade(duration)?;
        // The cache answered from the old value a moment ago and would go on
        // doing it: what the queue arms next has to be the new rule.
        *self.settings.write().unwrap_or_else(|err| err.into_inner()) = None;
        Ok(())
    }

    /// Puts the volume back where this profile left it.
    ///
    /// Called when a profile becomes the active one — at the start of a run and
    /// at a switch — because those are the two moments the engine is holding
    /// somebody else's level, or none at all. The engine has no database and no
    /// way to ask, the same as everything else here.
    ///
    /// A profile that has never touched it gets `Volume::default`, which is
    /// full: the first run of a music player should make a sound.
    pub fn restore_volume(&self) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        let volume = match self.context.settings.profile_get(profile_id, VOLUME_KEY)? {
            Some(value) => Volume::clamped(value.as_float()? as f32),
            None => Volume::default(),
        };

        *self.volume.write().unwrap_or_else(|err| err.into_inner()) = volume;
        // Through the same door a listener's own press goes through: muted
        // stays muted, and what the engine is told is what is heard.
        let muted = *self.muted.read().unwrap_or_else(|err| err.into_inner());
        let level = if muted { Volume::MUTED } else { volume };
        self.ports.engine.set_volume(level)?;
        self.announce();
        Ok(())
    }

    fn read_settings(&self, profile_id: ProfileId) -> Result<PlaybackSettings> {
        let defaults = PlaybackSettings::default();
        let store = &self.context.settings;

        let crossfade_enabled = match store.profile_get(profile_id, CROSSFADE_ENABLED_KEY)? {
            Some(value) => value.as_bool()?,
            None => defaults.crossfade_enabled,
        };
        let preload_next = match store.profile_get(profile_id, PRELOAD_NEXT_KEY)? {
            Some(value) => value.as_bool()?,
            None => defaults.preload_next,
        };
        let crossfade = match store.profile_get(profile_id, CROSSFADE_MS_KEY)? {
            Some(value) => {
                let millis = u64::try_from(value.as_integer()?)
                    .map_err(|_| CoreError::invalid("crossfade", "a length cannot be negative"))?;
                CrossfadeDuration::new(DurationMs::from_millis(millis))?
            }
            None => defaults.crossfade,
        };

        Ok(PlaybackSettings {
            crossfade_enabled,
            crossfade,
            preload_next,
        })
    }

    /// Loads a track from the active profile's library and starts it.
    pub fn play_track(&self, media_file_id: MediaFileId, source: PlaySource) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        // Before the engine loads anything: loading resets the position, and
        // the position is how much of the outgoing track was heard.
        self.close_listen(self.ports.engine.position().elapsed());

        // The listing and the file are two different questions: a row can exist
        // for a file that has since been deleted from disk.
        let summary = self
            .ports
            .tracks
            .summary(profile_id, media_file_id)?
            .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

        let media_file = self
            .ports
            .media_files
            .get(media_file_id)?
            .ok_or_else(|| CoreError::not_found("media file", media_file_id))?;

        if !media_file.state.is_playable() {
            return Err(CoreError::Audio(format!(
                "{} is {} — the library knows it but the file does not answer",
                media_file.path.display(),
                media_file.state.as_str()
            )));
        }

        self.ports.engine.load(&media_file.path)?;
        self.ports.engine.play()?;

        self.open_listen(&summary, source);
        self.write_loaded(Some(summary));
        self.announce();
        Ok(())
    }

    /// Resumes, or starts what is already loaded.
    pub fn resume(&self) -> Result<()> {
        self.ports.engine.play()?;
        self.announce();
        Ok(())
    }

    /// Holds position without unloading.
    pub fn pause(&self) -> Result<()> {
        self.ports.engine.pause()?;
        self.announce();
        Ok(())
    }

    /// Pauses if playing, resumes if not.
    ///
    /// One button, so one command: leaving the interface to decide which of two
    /// calls to make would put a rule about transport state in the layer that is
    /// not allowed to hold one.
    pub fn toggle(&self) -> Result<()> {
        match self.ports.engine.state() {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused => self.resume(),
            // Nothing loaded, or the track ran out. Starting it again from the
            // top is what pressing play on a finished track should do.
            PlaybackState::Stopped => match self.loaded_id() {
                // The source of a replay is the source of what is loaded, and
                // by this point that is no longer known — the listen it came
                // with has been written down. Library is the honest default:
                // the listener pressed play on a track, which is what the
                // library listing does.
                Some(media_file_id) => self.play_track(media_file_id, PlaySource::Library),
                None => Err(CoreError::Audio("nothing is loaded to play".into())),
            },
        }
    }

    /// Stops and unloads.
    pub fn stop(&self) -> Result<()> {
        self.close_listen(self.ports.engine.position().elapsed());

        self.ports.engine.stop()?;
        self.write_loaded(None);
        self.announce();
        Ok(())
    }

    /// Jumps to a position in the current track.
    pub fn seek(&self, position: PlaybackPosition) -> Result<()> {
        // A probe, while a reported defect is being hunted (MASTER_ISSUES 83).
        // Seeking with a transition armed is the state the defect needs, and it
        // cannot be reproduced in a test: the count of transitions is raised by
        // the audio callback crossing the middle of a crossfade, while a seek
        // travels to the decode thread as a command. Only a real machine has
        // both a callback and a queue of commands.
        //
        // Cheap enough to leave: a listener seeks a handful of times an hour.
        self.context.info(&format!(
            "seek to {} ms, armed = {}",
            position.as_millis(),
            self.ports.engine.armed()
        ));

        self.ports.engine.seek(position)?;
        self.announce();
        Ok(())
    }

    /// Sets the output level.
    ///
    /// Written down as well as applied, because a player that opens at full
    /// volume every morning is a player somebody turns down every morning
    /// (`MASTER_ISSUES` 98). Every step of a drag writes one small row; volume
    /// is not dragged often enough for that to be worth the preview-and-commit
    /// dance the equaliser needs.
    ///
    /// Setting a level while muted unmutes: reaching for the volume is how
    /// somebody says they want to hear something.
    pub fn set_volume(&self, volume: Volume) -> Result<()> {
        *self.volume.write().unwrap_or_else(|err| err.into_inner()) = volume;

        // A profile is needed to write it against and not to hear it: a window
        // open before anybody has chosen one still has a volume, it is simply
        // nobody's yet.
        if let Some(profile_id) = self.context.active_profile() {
            self.context.settings.profile_set(
                profile_id,
                VOLUME_KEY,
                &SettingValue::Float(f64::from(volume.as_f32())),
                self.context.now(),
            )?;
        }
        *self.muted.write().unwrap_or_else(|err| err.into_inner()) = false;

        self.ports.engine.set_volume(volume)?;
        self.announce();
        Ok(())
    }

    /// Silences output without forgetting the level, or restores it.
    pub fn toggle_mute(&self) -> Result<()> {
        let muted = {
            let mut flag = self.muted.write().unwrap_or_else(|err| err.into_inner());
            *flag = !*flag;
            *flag
        };

        let level = if muted { Volume::MUTED } else { self.volume() };
        self.ports.engine.set_volume(level)?;
        self.announce();
        Ok(())
    }

    /// Everything the player bar draws, read fresh from the engine.
    pub fn view(&self) -> PlayerView {
        let track = self
            .loaded
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone();

        PlayerView {
            state: self.ports.engine.state(),
            duration: track
                .as_ref()
                .map_or_else(Default::default, |summary| summary.duration),
            track,
            position: self.ports.engine.position(),
            volume: self.volume(),
            muted: *self.muted.read().unwrap_or_else(|err| err.into_inner()),
        }
    }

    /// The level the slider shows.
    fn volume(&self) -> Volume {
        *self.volume.read().unwrap_or_else(|err| err.into_inner())
    }

    fn loaded_id(&self) -> Option<MediaFileId> {
        self.loaded
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .map(|summary| summary.media_file_id)
    }

    /// Starts counting a listen, if this profile keeps history.
    ///
    /// Asked of the profile every time rather than cached: turning history off
    /// is a decision that must take effect at once, and a cached "yes" would
    /// keep writing for as long as the window stayed open
    /// (PROJECT_MASTER 1.4, 2.6).
    fn open_listen(&self, summary: &TrackSummary, source: PlaySource) {
        let keeping = self
            .context
            .active_profile()
            .and_then(|id| self.context.profiles.get(id).ok().flatten())
            .filter(should_record);

        *self
            .listening
            .write()
            .unwrap_or_else(|err| err.into_inner()) = keeping.map(|profile| Listen {
            profile_id: profile.id,
            media_file_id: summary.media_file_id,
            source,
            duration: summary.duration,
            started_at: self.context.now(),
        });
    }

    /// Writes down the listen that has just ended, if one was open.
    ///
    /// A failure to record is not a failure to play: the listener is listening,
    /// and a database that will not take a row about it has not stopped them.
    fn close_listen(&self, played: DurationMs) {
        let Some(listen) = self
            .listening
            .write()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        else {
            return;
        };

        // Never more than the track is long. A listener who seeks backwards and
        // hears a chorus twice has still heard one track's worth of it, and a
        // completion rate above one would be arithmetic nobody could explain.
        let played = played.min(listen.duration);

        let event = PlayEvent {
            id: PlayEventId::new(),
            profile_id: listen.profile_id,
            media_file_id: listen.media_file_id,
            source: listen.source,
            started_at: listen.started_at,
            ended_at: Some(self.context.now()),
            played,
            duration: listen.duration,
            outcome: classify(played, listen.duration),
        };

        // A listen that fails to record is a row missing from a month of
        // statistics, and stopping the music over it would be a worse answer to
        // a worse problem. Deliberate, and written down rather than swallowed.
        if let Err(err) = self.ports.history.append(&event) {
            self.context
                .warn(&format!("a listen was not recorded: {err}"));
        }
    }

    /// How long the track that is ending was heard for.
    ///
    /// Used where the engine has already moved on and cannot be asked: a join
    /// happens because the track ran out, so it was heard to its end.
    fn listened_duration(&self) -> DurationMs {
        self.listening
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .map_or(DurationMs::ZERO, |listen| listen.duration)
    }

    fn write_loaded(&self, summary: Option<TrackSummary>) {
        *self.loaded.write().unwrap_or_else(|err| err.into_inner()) = summary;
    }

    fn announce(&self) {
        self.context.events.publish(DomainEvent::PlaybackChanged);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{PlaybackPorts, PlaybackService};
    use crate::Result;
    use crate::application::context::AppContext;
    use crate::domain::ids::{MediaFileId, ProfileId};
    use crate::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
    use crate::domain::playback::{PlaybackState, TransitionProfile};
    use crate::domain::ports::audio_engine::AudioEnginePort;
    use crate::domain::ports::clock::ClockPort;
    use crate::domain::ports::event_bus::{DomainEvent, EventBusPort, EventHandler};
    use crate::domain::ports::repositories::{
        MediaFileRepositoryPort, PlayEventRepositoryPort, ProfileRepositoryPort,
        SettingsRepositoryPort, TrackRepositoryPort,
    };
    use crate::domain::profile::Profile;
    use crate::domain::settings::{ProfileFolder, SettingValue};
    use crate::domain::stats::{PlayEvent, PlaySource};
    use crate::domain::track::{Track, TrackSummary};
    use crate::domain::value_objects::{DurationMs, PlaybackPosition, Timestamp, Volume};

    use std::sync::Arc;

    /// An engine that records what it was told instead of making a sound.
    #[derive(Default)]
    struct FakeEngine {
        loaded: Mutex<Option<PathBuf>>,
        /// What was armed to follow, which a load throws away exactly as the
        /// real engine does.
        armed: Mutex<Option<PathBuf>>,
        playing: AtomicBool,
        volume: Mutex<Option<Volume>>,
        position: Mutex<PlaybackPosition>,
    }

    impl AudioEnginePort for FakeEngine {
        fn load(&self, path: &Path) -> Result<()> {
            *self.loaded.lock().expect("not poisoned") = Some(path.to_path_buf());
            *self.armed.lock().expect("not poisoned") = None;
            *self.position.lock().expect("not poisoned") = PlaybackPosition::START;
            Ok(())
        }
        fn preload_next(&self, path: &Path, _transition: TransitionProfile) -> Result<()> {
            *self.armed.lock().expect("not poisoned") = Some(path.to_path_buf());
            Ok(())
        }
        fn armed(&self) -> bool {
            self.armed.lock().expect("not poisoned").is_some()
        }
        fn advances(&self) -> u64 {
            0
        }
        fn play(&self) -> Result<()> {
            self.playing.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn pause(&self) -> Result<()> {
            self.playing.store(false, Ordering::Relaxed);
            Ok(())
        }
        fn stop(&self) -> Result<()> {
            self.playing.store(false, Ordering::Relaxed);
            *self.loaded.lock().expect("not poisoned") = None;
            Ok(())
        }
        fn seek(&self, position: PlaybackPosition) -> Result<()> {
            *self.position.lock().expect("not poisoned") = position;
            Ok(())
        }
        fn set_volume(&self, volume: Volume) -> Result<()> {
            *self.volume.lock().expect("not poisoned") = Some(volume);
            Ok(())
        }
        fn set_crossfade(
            &self,
            _duration: crate::domain::settings::CrossfadeDuration,
        ) -> Result<()> {
            Ok(())
        }
        fn set_eq(&self, _setting: &crate::domain::eq::EqSetting) -> Result<()> {
            Ok(())
        }
        fn set_visualising(&self, _on: bool) {}
        fn spectrum(&self, _bars: &mut [f32]) -> bool {
            false
        }
        fn position(&self) -> PlaybackPosition {
            *self.position.lock().expect("not poisoned")
        }
        fn state(&self) -> PlaybackState {
            if self.playing.load(Ordering::Relaxed) {
                PlaybackState::Playing
            } else if self.loaded.lock().expect("not poisoned").is_some() {
                PlaybackState::Paused
            } else {
                PlaybackState::Stopped
            }
        }
    }

    /// A catalogue holding exactly one file.
    struct OneFile {
        media_file: MediaFile,
    }

    impl MediaFileRepositoryPort for OneFile {
        fn get(&self, id: MediaFileId) -> Result<Option<MediaFile>> {
            Ok((id == self.media_file.id).then(|| self.media_file.clone()))
        }
        fn find_by_path(&self, _path: &Path) -> Result<Option<MediaFile>> {
            unreachable!("playback looks files up by identifier")
        }
        fn find_by_hash(&self, _hash: &str) -> Result<Vec<MediaFile>> {
            unreachable!("playback does not de-duplicate")
        }
        fn save(&self, _media_file: &MediaFile) -> Result<()> {
            unreachable!("playback does not write to the catalogue")
        }
        fn set_state(&self, _id: MediaFileId, _state: FileState, _now: Timestamp) -> Result<()> {
            unreachable!("playback does not write to the catalogue")
        }
        fn set_path(&self, _id: MediaFileId, _path: &Path, _now: Timestamp) -> Result<()> {
            unreachable!("playback does not move files")
        }
    }

    /// A library holding exactly one row.
    struct OneRow {
        summary: TrackSummary,
    }

    impl TrackRepositoryPort for OneRow {
        fn get(&self, _profile: ProfileId, _file: MediaFileId) -> Result<Option<Track>> {
            unreachable!("playback reads summaries")
        }
        fn list_for_profile(&self, _profile: ProfileId) -> Result<Vec<Track>> {
            unreachable!("playback reads summaries")
        }
        fn removed_for_profile(&self, _profile: ProfileId) -> Result<Vec<TrackSummary>> {
            unreachable!("playback does not look at what was taken out")
        }
        fn summaries_for_profile(&self, _profile: ProfileId) -> Result<Vec<TrackSummary>> {
            Ok(vec![self.summary.clone()])
        }
        fn summary(
            &self,
            _profile: ProfileId,
            media_file_id: MediaFileId,
        ) -> Result<Option<TrackSummary>> {
            Ok((media_file_id == self.summary.media_file_id).then(|| self.summary.clone()))
        }
        fn save(&self, _track: &Track) -> Result<()> {
            unreachable!("playback does not write to the library")
        }
        fn remove(&self, _p: ProfileId, _f: MediaFileId, _now: Timestamp) -> Result<()> {
            unreachable!("playback does not remove tracks")
        }
        fn restore(&self, _profile: ProfileId, _file: MediaFileId) -> Result<()> {
            unreachable!("playback does not restore tracks")
        }
    }

    struct FixedClock;
    impl ClockPort for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp::from_millis(1_700_000_000_000)
        }
    }

    #[derive(Default)]
    struct CountingBus {
        published: Mutex<Vec<DomainEvent>>,
    }
    impl EventBusPort for CountingBus {
        fn publish(&self, event: DomainEvent) {
            self.published.lock().expect("not poisoned").push(event);
        }
        fn subscribe(&self, _handler: EventHandler) {}
    }

    /// Playback never reads either of these; the context demands them.
    struct NoProfiles;
    impl ProfileRepositoryPort for NoProfiles {
        fn list(&self) -> Result<Vec<Profile>> {
            Ok(Vec::new())
        }
        fn get(&self, _id: ProfileId) -> Result<Option<Profile>> {
            Ok(None)
        }
        fn save(&self, _profile: &Profile) -> Result<()> {
            Ok(())
        }
        fn delete(&self, _id: ProfileId) -> Result<()> {
            Ok(())
        }
    }

    struct NoSettings;
    impl SettingsRepositoryPort for NoSettings {
        fn app_get(&self, _key: &str) -> Result<Option<SettingValue>> {
            Ok(None)
        }
        fn app_set(&self, _key: &str, _value: &SettingValue, _now: Timestamp) -> Result<()> {
            Ok(())
        }
        fn app_remove(&self, _key: &str) -> Result<()> {
            Ok(())
        }
        fn profile_get(&self, _p: ProfileId, _key: &str) -> Result<Option<SettingValue>> {
            Ok(None)
        }
        fn profile_set(
            &self,
            _p: ProfileId,
            _key: &str,
            _value: &SettingValue,
            _now: Timestamp,
        ) -> Result<()> {
            Ok(())
        }
        fn profile_remove(&self, _p: ProfileId, _key: &str) -> Result<()> {
            Ok(())
        }
        fn list_folders(&self, _profile_id: ProfileId) -> Result<Vec<ProfileFolder>> {
            Ok(Vec::new())
        }
        fn save_folder(&self, _folder: &ProfileFolder) -> Result<()> {
            Ok(())
        }
        fn delete_folder(&self, _folder: &ProfileFolder) -> Result<()> {
            Ok(())
        }
    }

    /// A service with one playable track, and the engine it talks to.
    fn service(state: FileState) -> (PlaybackService, Arc<FakeEngine>, MediaFileId) {
        let media_file_id = MediaFileId::new();
        let media_file = MediaFile {
            id: media_file_id,
            path: PathBuf::from("C:/music/mysterons.flac"),
            file_hash: None,
            file_size: 1_024,
            file_mtime: Timestamp::from_millis(0),
            format: AudioFormat::Flac,
            properties: AudioProperties {
                duration: DurationMs::from_secs(300),
                sample_rate: 44_100,
                channels: 2,
                bitrate: None,
            },
            metadata_version: None,
            metadata_extracted_at: None,
            state,
            created_at: Timestamp::from_millis(0),
            updated_at: Timestamp::from_millis(0),
        };

        let context = Arc::new(AppContext::new(
            Arc::new(FixedClock),
            Arc::new(CountingBus::default()),
            Arc::new(NoProfiles),
            Arc::new(NoSettings),
        ));
        context.set_active_profile(ProfileId::new());

        let engine = Arc::new(FakeEngine::default());
        let service = PlaybackService::new(
            context,
            PlaybackPorts {
                engine: Arc::clone(&engine) as Arc<dyn AudioEnginePort>,
                media_files: Arc::new(OneFile { media_file }),
                tracks: Arc::new(OneRow {
                    summary: TrackSummary {
                        media_file_id,
                        title: "Mysterons".to_owned(),
                        artist: Some("Portishead".to_owned()),
                        album: None,
                        duration: DurationMs::from_secs(300),
                    },
                }),
                history: Arc::new(NoHistory),
            },
        );

        (service, engine, media_file_id)
    }

    /// History that goes nowhere. These tests are about the transport, and the
    /// profile behind them has none anyway — `NoProfiles` answers nothing, so
    /// no listen is ever opened.
    struct NoHistory;

    impl PlayEventRepositoryPort for NoHistory {
        fn append(&self, _event: &PlayEvent) -> Result<()> {
            Ok(())
        }
        fn recent(
            &self,
            _profile_id: ProfileId,
            _since: Timestamp,
            _limit: u32,
        ) -> Result<Vec<PlayEvent>> {
            Ok(Vec::new())
        }
        fn purge_before(&self, _profile_id: ProfileId, _cutoff: Timestamp) -> Result<u64> {
            Ok(0)
        }
        fn purge_all(&self, _profile_id: ProfileId) -> Result<u64> {
            Ok(0)
        }
    }

    #[test]
    fn playing_a_track_loads_the_file_and_starts_it() {
        let (service, engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");

        assert_eq!(
            engine.loaded.lock().expect("not poisoned").as_deref(),
            Some(Path::new("C:/music/mysterons.flac"))
        );

        let view = service.view();
        assert_eq!(view.state, PlaybackState::Playing);
        assert_eq!(view.track.expect("a track").title, "Mysterons");
        assert_eq!(view.duration, DurationMs::from_secs(300));
    }

    #[test]
    fn a_missing_file_is_refused_with_its_path_rather_than_played() {
        let (service, engine, id) = service(FileState::Missing);

        let message = service
            .play_track(id, PlaySource::Library)
            .expect_err("not playable")
            .to_string();
        assert!(message.contains("mysterons.flac"), "got {message}");
        assert!(
            engine.loaded.lock().expect("not poisoned").is_none(),
            "the engine was never asked to open it"
        );
    }

    #[test]
    fn one_button_pauses_and_resumes() {
        let (service, _engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");

        service.toggle().expect("paused");
        assert_eq!(service.view().state, PlaybackState::Paused);

        service.toggle().expect("resumed");
        assert_eq!(service.view().state, PlaybackState::Playing);
    }

    #[test]
    fn pressing_play_on_a_finished_track_starts_it_again() {
        let (service, engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");

        // What the engine reports once a track has run out and been unloaded by
        // the listener, while the service still remembers what it was.
        engine.stop().expect("stopped");
        assert_eq!(service.view().state, PlaybackState::Stopped);

        service.toggle().expect("started again");
        assert_eq!(service.view().state, PlaybackState::Playing);
    }

    #[test]
    fn toggling_with_nothing_loaded_says_so() {
        let (service, _engine, _id) = service(FileState::Available);
        assert!(service.toggle().is_err(), "there is nothing to play");
    }

    #[test]
    fn muting_silences_the_output_without_forgetting_the_level() {
        let (service, engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");

        let chosen = Volume::new(0.4).expect("in range");
        service.set_volume(chosen).expect("set");

        service.toggle_mute().expect("muted");
        assert_eq!(
            *engine.volume.lock().expect("not poisoned"),
            Some(Volume::MUTED),
            "the engine is silent"
        );
        assert_eq!(service.view().volume, chosen, "the slider is not");
        assert!(service.view().muted);

        service.toggle_mute().expect("unmuted");
        assert_eq!(*engine.volume.lock().expect("not poisoned"), Some(chosen));
        assert!(!service.view().muted);
    }

    #[test]
    fn reaching_for_the_volume_unmutes() {
        let (service, _engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");
        service.toggle_mute().expect("muted");

        service
            .set_volume(Volume::new(0.7).expect("in range"))
            .expect("set");

        assert!(!service.view().muted, "asking to hear something unmutes it");
    }

    #[test]
    fn stopping_forgets_the_track() {
        let (service, _engine, id) = service(FileState::Available);
        service.play_track(id, PlaySource::Library).expect("played");
        service.stop().expect("stopped");

        let view = service.view();
        assert_eq!(view.state, PlaybackState::Stopped);
        assert!(view.track.is_none());
    }
}
