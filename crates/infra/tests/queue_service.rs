//! The queue drives playback: the rest of the M7 definition of done.
//!
//! Real repositories over a real database, and a fake audio device — the one
//! thing a test cannot have. What is under test is the order tracks start in
//! and what survives a restart, neither of which needs a speaker.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cadenza_core::Result;
use cadenza_core::application::services::{
    PlaybackPorts, PlaybackService, QueuePorts, QueueService,
};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::repositories::{MediaFileRepositoryPort, TrackRepositoryPort};
use cadenza_core::domain::queue::RepeatMode;
use cadenza_core::domain::settings::CrossfadeDuration;
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Timestamp, Volume};
use cadenza_infra::db::repositories::{
    SqliteMediaFileRepository, SqliteProfileRepository, SqliteQueueRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
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

    /// Runs the current track out.
    fn finish(&self) {
        self.ended.store(true, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }
}

impl AudioEnginePort for FakeEngine {
    fn load(&self, path: &Path) -> Result<()> {
        *self.loaded.lock().expect("not poisoned") = Some(path.to_path_buf());
        self.started
            .lock()
            .expect("not poisoned")
            .push(path.to_path_buf());
        *self.position.lock().expect("not poisoned") = PlaybackPosition::START;
        self.ended.store(false, Ordering::Relaxed);
        Ok(())
    }
    fn preload_next(&self, _path: &Path, _transition: TransitionProfile) -> Result<()> {
        Ok(())
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
    db: TempDb,
    queue: QueueService,
    engine: Arc<FakeEngine>,
    profile_id: ProfileId,
    tracks: Vec<MediaFileId>,
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

    let (queue, engine) = services(&db, profile.id);

    Harness {
        db,
        queue,
        engine,
        profile_id: profile.id,
        tracks,
    }
}

/// The queue and the device under it, as one run of the application builds them.
fn services(db: &TempDb, profile_id: ProfileId) -> (QueueService, Arc<FakeEngine>) {
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
        },
    ));

    let queue = QueueService::new(
        context,
        playback,
        QueuePorts {
            queue: Arc::new(SqliteQueueRepository::new(db.pool().clone())),
            tracks: track_repo as _,
        },
    );

    (queue, engine)
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
fn choosing_a_row_queues_the_rest_of_the_library_behind_it() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    assert_eq!(harness.engine.heard(), vec!["one"]);
    assert!(
        !harness.listed().is_empty(),
        "choosing a track is choosing a starting point"
    );

    let view = harness.queue.view();
    assert!(view.has_next);
    assert!(!view.has_previous, "nothing played before it");
}

#[test]
fn a_track_that_runs_out_starts_the_next_one() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    let next = harness.listed().first().cloned().expect("something queued");

    harness.engine.finish();
    harness.queue.poll().expect("polled");

    assert_eq!(
        harness.engine.heard(),
        vec!["one".to_owned(), next.clone()],
        "the queue advanced on its own"
    );
    assert!(harness.queue.view().has_previous);
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

    // Four tracks: three advances empty the queue, the fourth stops.
    for _ in 0..4 {
        harness.engine.finish();
        harness.queue.poll().expect("polled");
    }

    assert_eq!(harness.engine.heard().len(), 4, "each track played once");
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
fn a_manually_queued_track_plays_before_the_continuation() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    // The last track in library order, so it would otherwise play last.
    harness.queue.enqueue(harness.tracks[3]).expect("queued");
    assert_eq!(
        harness.listed().first().map(String::as_str),
        Some("four"),
        "the manual queue is at the front"
    );

    harness.engine.finish();
    harness.queue.poll().expect("polled");
    assert_eq!(harness.engine.heard(), vec!["one", "four"]);
}

#[test]
fn shuffle_reorders_what_is_still_to_play_and_is_remembered() {
    let harness = harness();
    harness
        .queue
        .play_from_library(harness.tracks[0])
        .expect("played");

    let ordered = harness.listed();
    harness.queue.toggle_shuffle().expect("shuffled");
    assert!(harness.queue.view().shuffle);

    let shuffled = harness.listed();
    let mut sorted = shuffled.clone();
    sorted.sort();
    let mut expected = ordered.clone();
    expected.sort();
    assert_eq!(sorted, expected, "the same tracks, in some order");

    harness.queue.toggle_shuffle().expect("unshuffled");
    assert_eq!(
        harness.listed(),
        ordered,
        "turning it off restores library order"
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
