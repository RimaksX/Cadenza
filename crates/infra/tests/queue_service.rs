//! The queue drives playback: the rest of the M7 definition of done.
//!
//! Real repositories over a real database, and a fake audio device — the one
//! thing a test cannot have. What is under test is the order tracks start in
//! and what survives a restart, neither of which needs a speaker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cadenza_core::Result;
use cadenza_core::application::services::{
    PlaybackPorts, PlaybackService, QueuePorts, QueueService, RadioPorts, RadioService,
};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::eq::EqSetting;
use cadenza_core::domain::ids::{MediaFileId, PlaylistId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::repositories::TrackFeaturesRepositoryPort;
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, SettingsRepositoryPort, TrackRepositoryPort,
};
use cadenza_core::domain::queue::RepeatMode;
use cadenza_core::domain::radio::MIN_BATCH_SIZE;
use cadenza_core::domain::settings::{CROSSFADE_ENABLED_KEY, CrossfadeDuration, SettingValue};
use cadenza_core::domain::track::{Track, TrackFeatures};
use cadenza_core::domain::value_objects::{
    Bpm, DurationMs, Mode, MusicalKey, PlaybackPosition, Timestamp, Volume,
};
use cadenza_infra::db::repositories::{
    SqliteHistoryRepository, SqliteMediaFileRepository, SqliteMoodRepository,
    SqliteProfileRepository, SqliteQueueRepository, SqliteRadioRepository,
    SqliteSettingsRepository, SqliteTrackFeaturesRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

/// An engine that records what it was told to play and can be made to reach the
/// end of a track on command.
#[derive(Default)]
struct FakeEngine {
    loaded: Mutex<Option<PathBuf>>,
    started: Mutex<Vec<PathBuf>>,
    playing: AtomicBool,
    /// Set by the test to mean "the decoder ran out and the ring emptied",
    /// which is the one state the real engine reports as stopped-but-loaded.
    ended: AtomicBool,
    position: Mutex<PlaybackPosition>,
    /// What has been armed to follow, if anything, and how it would arrive.
    armed: Mutex<Option<PathBuf>>,
    armed_transition: Mutex<Option<TransitionProfile>>,
    /// How many times a track was loaded outright, which is the thing a join
    /// must not do.
    loads: AtomicU64,
    /// How many joins have been made without anybody asking.
    advances: AtomicU64,
}

impl FakeEngine {
    /// What the listener would have heard, in order.
    fn heard(&self) -> Vec<String> {
        self.started
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|path| {
                path.file_stem()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    /// How the armed track would arrive, if one is armed.
    fn armed_transition(&self) -> Option<TransitionProfile> {
        *self.armed_transition.lock().expect("not poisoned")
    }

    /// Which track is open to follow this one.
    fn armed_name(&self) -> Option<String> {
        self.armed
            .lock()
            .expect("not poisoned")
            .as_ref()
            .map(|path| {
                path.file_stem()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned()
            })
    }

    /// Runs the current track out.
    fn finish(&self) {
        self.ended.store(true, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }

    /// Plays out the join the engine had armed, the way the real one does.
    ///
    /// Nothing stops and nothing is loaded: the armed track simply becomes the
    /// one being heard, and the count says it happened. A caller that only
    /// watches for silence would never notice.
    fn hand_over(&self) {
        let Some(path) = self.armed.lock().expect("not poisoned").take() else {
            panic!("nothing was armed to hand over to");
        };
        self.started
            .lock()
            .expect("not poisoned")
            .push(path.clone());
        *self.loaded.lock().expect("not poisoned") = Some(path);
        *self.position.lock().expect("not poisoned") = PlaybackPosition::START;
        self.advances.fetch_add(1, Ordering::Relaxed);
    }
}

impl AudioEnginePort for FakeEngine {
    fn load(&self, path: &Path) -> Result<()> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        *self.loaded.lock().expect("not poisoned") = Some(path.to_path_buf());
        *self.armed.lock().expect("not poisoned") = None;
        self.started
            .lock()
            .expect("not poisoned")
            .push(path.to_path_buf());
        *self.position.lock().expect("not poisoned") = PlaybackPosition::START;
        self.ended.store(false, Ordering::Relaxed);
        Ok(())
    }
    fn preload_next(&self, path: &Path, transition: TransitionProfile) -> Result<()> {
        *self.armed.lock().expect("not poisoned") = Some(path.to_path_buf());
        *self.armed_transition.lock().expect("not poisoned") = Some(transition);
        Ok(())
    }
    fn armed(&self) -> bool {
        self.armed.lock().expect("not poisoned").is_some()
    }
    fn advances(&self) -> u64 {
        self.advances.load(Ordering::Relaxed)
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
    fn set_volume(&self, _volume: Volume) -> Result<()> {
        Ok(())
    }
    fn set_crossfade(&self, _duration: CrossfadeDuration) -> Result<()> {
        Ok(())
    }
    fn set_eq(&self, _setting: &EqSetting) -> Result<()> {
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
        if self.loaded.lock().expect("not poisoned").is_none() || self.ended.load(Ordering::Relaxed)
        {
            return PlaybackState::Stopped;
        }
        if self.playing.load(Ordering::Relaxed) {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }
}

/// A profile with a four-track library, wired the way the application wires it.
struct Harness {
    /// The one context everything shares, as the application has it: the active
    /// profile is a single cell, and a service reading a different copy of it
    /// would never notice a switch.
    context: Arc<AppContext>,
    queue: QueueService,
    engine: Arc<FakeEngine>,
    profile_id: ProfileId,
    tracks: Vec<MediaFileId>,
    /// Declared last on purpose: fields are dropped in declaration order, and
    /// the fixture cannot delete its directory while anything above it still
    /// holds a connection to the database inside it.
    db: TempDb,
}

fn harness() -> Harness {
    let db = TempDb::new();

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");
    context.set_active_profile(profile.id);

    let media_files = SqliteMediaFileRepository::new(db.pool().clone());
    let track_repo = SqliteTrackRepository::new(db.pool().clone());

    let tracks = ["one", "two", "three", "four"]
        .into_iter()
        .map(|name| {
            let media_file = catalogued(name);
            media_files.save(&media_file).expect("catalogued");
            track_repo
                .save(&in_library(profile.id, media_file.id, name))
                .expect("in the library");
            media_file.id
        })
        .collect();

    let (queue, engine, _radio, context) = services_with_radio(&db, profile.id);

    Harness {
        context,
        db,
        queue,
        engine,
        profile_id: profile.id,
        tracks,
    }
}

/// The queue and the device under it, as one run of the application builds them.
fn services(db: &TempDb, profile_id: ProfileId) -> (QueueService, Arc<FakeEngine>) {
    let (queue, engine, _radio, _context) = services_with_radio(db, profile_id);
    (queue, engine)
}

/// The same, with the station the application wires behind it.
fn services_with_radio(
    db: &TempDb,
    profile_id: ProfileId,
) -> (
    QueueService,
    Arc<FakeEngine>,
    Arc<RadioService>,
    Arc<AppContext>,
) {
    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    context.set_active_profile(profile_id);

    let track_repo = Arc::new(SqliteTrackRepository::new(db.pool().clone()));
    let engine = Arc::new(FakeEngine::default());

    let playback = Arc::new(PlaybackService::new(
        Arc::clone(&context),
        PlaybackPorts {
            engine: Arc::clone(&engine) as _,
            media_files: Arc::new(SqliteMediaFileRepository::new(db.pool().clone())),
            tracks: Arc::clone(&track_repo) as _,
            history: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    ));

    let radio = Arc::new(RadioService::new(
        Arc::clone(&context),
        RadioPorts {
            radio: Arc::new(SqliteRadioRepository::new(db.pool().clone())),
            moods: Arc::new(SqliteMoodRepository::new(db.pool().clone())),
            tracks: Arc::clone(&track_repo) as _,
            features: Arc::new(SqliteTrackFeaturesRepository::new(db.pool().clone())),
            stats: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    ));

    let queue = QueueService::new(
        Arc::clone(&context),
        playback,
        QueuePorts {
            queue: Arc::new(SqliteQueueRepository::new(db.pool().clone())),
            tracks: track_repo as _,
            features: Arc::new(SqliteTrackFeaturesRepository::new(db.pool().clone())),
            radio: Some(Arc::clone(&radio)),
        },
    );

    (queue, engine, radio, context)
}

fn catalogued(name: &str) -> MediaFile {
    MediaFile {
        id: MediaFileId::new(),
        path: PathBuf::from(format!("C:/music/{name}.flac")),
        file_hash: None,
        file_size: 1_024,
        file_mtime: Timestamp::from_millis(0),
        format: AudioFormat::Flac,
        properties: AudioProperties {
            duration: DurationMs::from_secs(200),
            sample_rate: 44_100,
            channels: 2,
            bitrate: None,
        },
        metadata_version: None,
        metadata_extracted_at: None,
        state: FileState::Available,
        created_at: Timestamp::from_millis(0),
        updated_at: Timestamp::from_millis(0),
    }
}

fn in_library(profile_id: ProfileId, media_file_id: MediaFileId, title: &str) -> Track {
    Track {
        profile_id,
        media_file_id,
        title: title.to_owned(),
        artist_id: None,
        album_id: None,
        track_no: None,
        disc_no: None,
        year: None,
        // Listings are ordered by title, so the fixture's names are its order:
        // four, one, three, two.
        added_at: Timestamp::from_millis(0),
        removed_at: None,
    }
}

impl Harness {
    /// The library in the order a listing shows it.
    fn listed(&self) -> Vec<String> {
        self.queue
            .upcoming()
            .expect("a queue")
            .into_iter()
            .map(|summary| summary.title)
            .collect()
    }
}

#[test]
fn choosing_a_row_queues_nothing_and_still_has_somewhere_to_go() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    assert_eq!(harness.engine.heard(), vec!["one"]);
    assert!(
        harness.listed().is_empty(),
        "the queue is the listener's own list, and nobody put anything in it"
    );

    let view = harness.queue.view();
    assert_eq!(view.pending, 0);
    assert!(view.has_next, "the library carries on behind it");
    assert!(!view.has_previous, "nothing played before it");
}

#[test]
fn a_track_that_runs_out_starts_the_next_one_in_the_library() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    harness.engine.finish();
    harness.queue.poll().expect("polled");

    // Listings are ordered by title — four, one, three, two — so what follows
    // "one" is "three", and it follows without ever having been queued.
    assert_eq!(
        harness.engine.heard(),
        vec!["one", "three"],
        "the library carried on by itself"
    );
    assert!(harness.listed().is_empty(), "and the queue stayed empty");
    assert!(harness.queue.view().has_previous);
}

#[test]
fn clearing_the_queue_leaves_the_music_playing() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    harness.queue.enqueue(harness.tracks[2]).expect("queued");

    harness.queue.clear().expect("cleared");

    assert!(harness.listed().is_empty(), "the list went");
    assert_eq!(harness.engine.heard(), vec!["one"], "the music did not");
    assert!(
        harness
            .engine
            .loaded
            .lock()
            .expect("not poisoned")
            .is_some(),
        "the track that was playing is still loaded"
    );

    // And what was playing is still part of the library, so the library still
    // carries on behind it.
    harness.engine.finish();
    harness.queue.poll().expect("polled");
    assert_eq!(harness.engine.heard(), vec!["one", "three"]);
}

#[test]
fn polling_a_playing_track_changes_nothing() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    for _ in 0..8 {
        harness.queue.poll().expect("polled");
    }
    assert_eq!(harness.engine.heard(), vec!["one"], "still the same track");
}

#[test]
fn the_queue_stops_at_the_end_unless_repeat_says_otherwise() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    // Starting at "one" leaves three rows below it: two advances reach the
    // bottom of the library, and the third has nowhere to go.
    for _ in 0..3 {
        harness.engine.finish();
        harness.queue.poll().expect("polled");
    }

    assert_eq!(
        harness.engine.heard(),
        vec!["one", "three", "two"],
        "it played to the bottom of the list and stopped there"
    );
    assert!(
        harness
            .engine
            .loaded
            .lock()
            .expect("not poisoned")
            .is_none(),
        "and the player stopped rather than holding a finished track"
    );
}

#[test]
fn repeat_all_starts_the_library_again() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.cycle_repeat().expect("repeat all");
    assert_eq!(harness.queue.view().repeat, RepeatMode::All);

    for _ in 0..4 {
        harness.engine.finish();
        harness.queue.poll().expect("polled");
    }

    let heard = harness.engine.heard();
    assert_eq!(heard.len(), 5, "the list came round again");
    assert_eq!(heard[4], heard[0], "starting where it started");
}

#[test]
fn repeat_one_plays_the_same_track_again() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.cycle_repeat().expect("all");
    harness.queue.cycle_repeat().expect("one");
    assert_eq!(harness.queue.view().repeat, RepeatMode::One);

    harness.engine.finish();
    harness.queue.poll().expect("polled");

    assert_eq!(harness.engine.heard(), vec!["one", "one"]);
}

#[test]
fn previous_restarts_the_track_until_it_is_early_enough_to_go_back() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.engine.finish();
    harness.queue.poll().expect("polled");

    let second = harness.engine.heard().last().cloned().expect("two tracks");

    // Well into the track: previous means "start this one again".
    harness
        .engine
        .seek(PlaybackPosition::from_secs(30))
        .expect("seeked");
    harness.queue.previous().expect("previous");
    assert_eq!(harness.engine.heard().len(), 2, "nothing new was loaded");
    assert_eq!(harness.engine.position(), PlaybackPosition::START);

    // Near the start: previous means the track before.
    harness.queue.previous().expect("previous");
    let heard = harness.engine.heard();
    assert_eq!(heard.len(), 3);
    assert_eq!(heard[2], "one", "back to what played first");
    assert_ne!(heard[2], second);
}

#[test]
fn a_manually_queued_track_plays_before_the_library_carries_on() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    // "three" is what the library would have played next. "four" is what was
    // asked for, and being asked for is what puts it first.
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    assert_eq!(harness.listed(), vec!["four"]);

    harness.engine.finish();
    harness.queue.poll().expect("polled");
    assert_eq!(harness.engine.heard(), vec!["one", "four"]);
    assert!(
        harness.listed().is_empty(),
        "and it left the queue when it started"
    );
}

#[test]
fn an_entry_can_be_taken_out_of_the_queue_by_where_it_is_shown() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    for track in &harness.tracks[1..4] {
        harness.queue.enqueue(*track).expect("queued");
    }

    let waiting = harness.listed();
    assert_eq!(waiting.len(), 3, "three tracks queued by hand");

    harness.queue.remove_at(1).expect("removed");

    assert_eq!(
        harness.listed(),
        vec![waiting[0].clone(), waiting[2].clone()],
        "the row that was pointed at is the row that went"
    );
    assert_eq!(
        harness.engine.heard(),
        vec!["one"],
        "and nothing started or stopped"
    );
}

#[test]
fn removing_counts_the_manual_queue_first_because_that_is_how_it_is_drawn() {
    let harness = harness();

    // A playlist is the one continuation the queue holds itself, so this is
    // where the two lanes can be told apart.
    let library = SqliteTrackRepository::new(harness.db.pool().clone())
        .summaries_for_profile(harness.profile_id)
        .expect("a library");
    harness
        .queue
        .play_playlist(PlaylistId::new(), &library, harness.tracks[0])
        .expect("played");
    harness.queue.enqueue(harness.tracks[3]).expect("queued");

    // The manual entry is drawn at the top, so position 0 is that one and not
    // the first of the continuation.
    harness.queue.remove_at(0).expect("removed");

    assert_eq!(
        harness.listed(),
        vec!["three", "two", "four"],
        "the manually queued copy went; the library's own copy is still last"
    );
    assert_eq!(harness.queue.view().pending, 3);
}

#[test]
fn choosing_a_row_in_the_queue_jumps_to_it_and_keeps_the_rest() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    for track in &harness.tracks[1..4] {
        harness.queue.enqueue(*track).expect("queued");
    }

    let waiting = harness.listed();
    assert_eq!(waiting.len(), 3);

    // The second row of the queue: one track is stepped over on the way.
    harness.queue.play_at(1).expect("jumped");

    assert_eq!(
        harness.engine.heard(),
        vec!["one".to_owned(), waiting[1].clone()],
        "it started the track that was pointed at"
    );
    assert_eq!(
        harness.listed(),
        vec![waiting[2].clone()],
        "and what was after it is still after it"
    );

    let view = harness.queue.view();
    assert!(
        view.has_previous,
        "the track that was skipped is what previous walks back through"
    );
}

#[test]
fn removing_a_position_that_is_not_there_says_so() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    assert!(
        harness.queue.remove_at(99).is_err(),
        "a queue that quietly ignores a removal is one that looks broken"
    );
}

#[test]
fn shuffle_plays_the_library_in_some_order_without_repeating_itself() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.toggle_shuffle().expect("shuffled");
    assert!(harness.queue.view().shuffle);

    for _ in 0..3 {
        harness.engine.finish();
        harness.queue.poll().expect("polled");
        assert!(
            harness.listed().is_empty(),
            "shuffle fills nothing in either"
        );
    }

    let mut heard = harness.engine.heard();
    assert_eq!(heard.len(), 4, "every track had a turn");
    assert_eq!(heard[0], "one", "starting where it was told to");
    heard.sort();
    assert_eq!(
        heard,
        vec!["four", "one", "three", "two"],
        "and none of them twice"
    );
}

#[test]
fn shuffle_does_not_jump_the_queue() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.toggle_shuffle().expect("shuffled");
    harness.queue.enqueue(harness.tracks[3]).expect("queued");

    harness.engine.finish();
    harness.queue.poll().expect("polled");

    assert_eq!(
        harness.engine.heard(),
        vec!["one", "four"],
        "what was asked for plays next; chance governs only the rest"
    );
}

#[test]
fn the_queue_is_still_there_after_a_restart() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.cycle_repeat().expect("repeat all");
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    harness.queue.enqueue(harness.tracks[2]).expect("queued");
    harness.engine.finish();
    harness.queue.poll().expect("polled");

    let before = harness.queue.view();
    let waiting = harness.listed();

    // What the next run of the application sees: a new service over the same
    // database and the same profile.
    let (restarted, _engine) = services(&harness.db, harness.profile_id);

    assert_eq!(restarted.view(), before, "modes and counts");
    assert_eq!(
        restarted
            .upcoming()
            .expect("a queue")
            .into_iter()
            .map(|summary| summary.title)
            .collect::<Vec<_>>(),
        waiting,
        "and what is still to play"
    );
}

#[test]
fn the_track_that_follows_is_opened_before_it_is_needed() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    assert!(
        !harness.engine.armed(),
        "nothing is armed until somebody looks"
    );

    harness.queue.poll().expect("polled");

    assert!(
        harness.engine.armed(),
        "the join is decoded ahead of itself"
    );
    assert_eq!(
        harness.engine.heard(),
        vec!["one"],
        "and nothing else has been started"
    );
}

#[test]
fn a_join_moves_the_queue_on_without_starting_anything() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.poll().expect("polled");

    // The engine plays the join out by itself. Nothing stops, so the state the
    // old end-of-track check watches for never happens.
    harness.engine.hand_over();
    assert!(harness.queue.poll().expect("polled"), "the queue caught up");

    assert_eq!(
        harness.engine.heard(),
        vec!["one", "three"],
        "the queue moved to the track the engine had already opened"
    );
    assert_eq!(
        harness.engine.loads.load(Ordering::Relaxed),
        1,
        "and it was never loaded — loading it would have cut the join in half"
    );
    assert!(
        harness.queue.view().has_previous,
        "the track that handed over is history"
    );
}

#[test]
fn a_track_queued_after_the_join_was_opened_still_plays_next() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.poll().expect("polled");
    assert_eq!(
        harness.engine.armed_name().as_deref(),
        Some("three"),
        "the library's own successor was opened ahead of itself"
    );

    // Half way through the track the listener asks for something else. What was
    // decoded is no longer what follows, and the engine has to be told.
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    harness.queue.poll().expect("polled");
    assert_eq!(harness.engine.armed_name().as_deref(), Some("four"));

    harness.engine.hand_over();
    harness.queue.poll().expect("polled");
    assert_eq!(
        harness.engine.heard(),
        vec!["one", "four"],
        "what the engine played is what the queue says played"
    );
}

#[test]
fn the_transition_follows_what_is_playing_rather_than_the_switch_alone() {
    let harness = harness();

    // Crossfade on for this profile. PROJECT_MASTER 2.4 makes that a statement
    // about ordinary tracks, not about everything.
    let settings = SqliteSettingsRepository::new(harness.db.pool().clone());
    settings
        .profile_set(
            harness.profile_id,
            CROSSFADE_ENABLED_KEY,
            &SettingValue::Bool(true),
            Timestamp::from_millis(0),
        )
        .expect("stored");

    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.poll().expect("polled");
    assert_eq!(
        harness.engine.armed_transition(),
        Some(TransitionProfile::Crossfade),
        "an ordinary track fades"
    );

    let library = SqliteTrackRepository::new(harness.db.pool().clone())
        .summaries_for_profile(harness.profile_id)
        .expect("a library");
    harness
        .queue
        .play_playlist(PlaylistId::new(), &library, harness.tracks[0])
        .expect("played");
    harness.queue.poll().expect("polled");
    assert_eq!(
        harness.engine.armed_transition(),
        Some(TransitionProfile::Gapless),
        "a playlist is continuous material and stays gapless"
    );
}

/// Features for one file: a tempo and an energy, and neutral everything else.
fn analysed(media_file_id: MediaFileId, bpm: f32, energy: f32, key: (u8, Mode)) -> TrackFeatures {
    TrackFeatures {
        media_file_id,
        bpm: Some(Bpm::new(bpm).expect("in range")),
        bpm_confidence: 1.0,
        key: Some(MusicalKey::new(key.0, key.1).expect("a key")),
        energy,
        loudness: 0.5,
        spectral_centroid: 0.5,
        spectral_rolloff: 0.5,
        danceability: 0.5,
        valence: energy,
        tempo_stability: 1.0,
        dynamic_range: 0.5,
        extractor_version: "test".to_owned(),
        analyzed_at: Timestamp::from_millis(0),
    }
}

/// One round: a fresh library, features, shuffle on, and one track played out.
///
/// A fresh one each time because the round is what has *not* been heard, and a
/// library of four is exhausted after two tracks. Twenty draws from the same
/// harness would be one real draw and nineteen forced ones.
fn who_follows_the_first_track() -> String {
    let harness = harness();
    let features = SqliteTrackFeaturesRepository::new(harness.db.pool().clone());

    // "one" is what plays, and "three" is beside it in every term the score
    // looks at. The other two are as far away as it can put them: the wrong
    // tempo, the wrong end of the energy range, and a key a tritone off in the
    // other mode.
    features
        .save(&analysed(harness.tracks[0], 120.0, 0.6, (0, Mode::Major)))
        .expect("saved");
    features
        .save(&analysed(harness.tracks[2], 122.0, 0.62, (0, Mode::Major)))
        .expect("saved");
    features
        .save(&analysed(harness.tracks[1], 200.0, 0.0, (6, Mode::Minor)))
        .expect("saved");
    features
        .save(&analysed(harness.tracks[3], 45.0, 1.0, (6, Mode::Minor)))
        .expect("saved");

    harness.queue.toggle_shuffle().expect("shuffled");
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.engine.finish();
    harness.queue.poll().expect("polled");

    harness
        .engine
        .heard()
        .last()
        .cloned()
        .expect("something followed")
}

#[test]
fn shuffle_prefers_the_track_that_follows_best_without_insisting_on_it() {
    // Counted rather than asserted per draw. The pick is weighted, not decided:
    // with these features the good transition takes about seven draws in ten,
    // and a test that demanded it every time would be testing for the very
    // behaviour PROJECT_MASTER 9.1 rules out. What is stable — and what the
    // milestone is actually about — is which one wins most often.
    //
    // A hundred draws rather than forty, because forty was not enough to make
    // the assertion below safe. Seven in ten over forty draws averages
    // twenty-eight wins with a standard deviation near three, and the test
    // demands more than twenty — under three deviations, which is roughly one
    // run in two hundred failing for no reason at all. It did, once, in a
    // full-workspace run. Over a hundred draws the same margin is four and a
    // half deviations, and a suite that cries wolf is a suite people stop
    // reading.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..100 {
        *counts.entry(who_follows_the_first_track()).or_default() += 1;
    }

    let close = counts.get("three").copied().unwrap_or_default();
    let others: usize = counts
        .iter()
        .filter(|(name, _)| name.as_str() != "three")
        .map(|(_, count)| *count)
        .sum();

    assert!(
        close > others,
        "the smooth transition should win more often than everything else          together: {counts:?}"
    );
    assert!(
        others > 0,
        "but never winning would mean shuffle had stopped being shuffle: {counts:?}"
    );
}

#[test]
fn shuffle_still_works_when_nothing_has_been_analysed() {
    let harness = harness();
    harness.queue.toggle_shuffle().expect("shuffled");
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    for _ in 0..3 {
        harness.engine.finish();
        harness.queue.poll().expect("polled");
    }

    let mut heard = harness.engine.heard();
    assert_eq!(heard.len(), 4, "every track had a turn");
    heard.sort();
    assert_eq!(
        heard,
        vec!["four", "one", "three", "two"],
        "and none of them twice"
    );
}

#[test]
fn playing_a_track_ends_the_station() {
    let harness = harness();
    let (queue, engine, radio, _context) = services_with_radio(&harness.db, harness.profile_id);

    let workout = radio
        .moods()
        .expect("moods")
        .into_iter()
        .find(|mood| mood.name == "Workout")
        .expect("a built-in mood");

    let session = radio.start(workout.id, None).expect("a station");
    let batch = radio.next_batch(MIN_BATCH_SIZE).expect("a batch");
    queue.play_radio(session.id, &batch).expect("playing");
    assert!(radio.session().is_some(), "the station is on");

    // What the listener did: went to the library and put something on.
    queue
        .play_from_library(harness.tracks[0])
        .expect("their own choice");

    assert!(
        radio.session().is_none(),
        "choosing a track is how a listener says they are done with the station"
    );
    assert_eq!(engine.heard().last().map(String::as_str), Some("one"));

    // And the tick that follows must not ask the station it just ended for
    // more, however low the lane it left behind has run.
    for _ in 0..4 {
        queue
            .poll()
            .expect("polled without asking a station that ended");
    }
}

#[test]
fn switching_listener_puts_the_other_ones_queue_away() {
    let harness = harness();
    // The same context the queue reads, because the active profile is one cell
    // and this test is about noticing that it moved.
    let profiles = ProfileService::new(Arc::clone(&harness.context));

    // Sasha queues two tracks by hand and starts one.
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    harness.queue.enqueue(harness.tracks[2]).expect("queued");
    assert_eq!(harness.listed(), vec!["four", "three"]);

    // Somebody else takes over. Stopping playback is the first of 2.5's three
    // steps and belongs to the transport; what is under test here is the third
    // — that the queue that comes back is the one belonging to whoever is now
    // listening.
    let other = profiles.create("Alex").expect("a second listener");
    profiles.switch_to(other.id).expect("switched");
    harness.queue.reload();

    assert!(
        harness.listed().is_empty(),
        "Alex has never queued anything: {:?}",
        harness.listed()
    );
    assert_eq!(harness.queue.view().pending, 0);
    assert!(!harness.queue.view().has_previous, "nor played anything");

    // And back again: what Sasha left is still hers.
    profiles
        .switch_to(harness.profile_id)
        .expect("switched back");
    harness.queue.reload();
    assert_eq!(harness.listed(), vec!["four", "three"]);
}
