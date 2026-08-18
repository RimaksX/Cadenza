//! Listening history, over a real database.
//!
//! What M14 is judged on: the statistics are counted, the history is cleared,
//! and — the rule that outranks both — nothing is written at all for a listener
//! who has turned it off (PROJECT_MASTER 1.4, 2.6).

use std::sync::Arc;

use cadenza_core::application::services::{PlaybackPorts, PlaybackService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::policies::retention_policy::cutoff;
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, PlayEventRepositoryPort, StatsRepositoryPort, TrackRepositoryPort,
};
use cadenza_core::domain::profile::HISTORY_RETENTION_DAYS;
use cadenza_core::domain::stats::{PlayOutcome, PlaySource};
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteHistoryRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

mod fake_engine;
use fake_engine::FakeEngine;

/// A three-minute track, which is long enough for half of it to mean something.
const TRACK: DurationMs = DurationMs::from_secs(180);

struct Harness {
    context: Arc<AppContext>,
    pool: cadenza_infra::db::SqlitePool,
    playback: PlaybackService,
    history: SqliteHistoryRepository,
    engine: Arc<FakeEngine>,
    profiles: ProfileService,
    profile_id: ProfileId,
    tracks: Vec<MediaFileId>,
    _db: TempDb,
}

fn harness() -> Harness {
    let db = TempDb::new();

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    let profiles = ProfileService::new(Arc::clone(&context));
    let profile = profiles.create("Sasha").expect("a profile");
    context.set_active_profile(profile.id);

    // History is off for a new profile — privacy is the default (1.4) — so a
    // test about what gets written has to ask for it first.
    profiles
        .set_history_enabled(profile.id, true)
        .expect("history on");

    let media_files = SqliteMediaFileRepository::new(db.pool().clone());
    let track_repo = SqliteTrackRepository::new(db.pool().clone());

    let tracks = ["one", "two"]
        .into_iter()
        .map(|name| {
            let media_file = MediaFile {
                id: MediaFileId::new(),
                path: format!("C:/music/{name}.flac").into(),
                file_hash: None,
                file_size: 1_024,
                file_mtime: Timestamp::from_millis(0),
                format: AudioFormat::Flac,
                properties: AudioProperties {
                    duration: TRACK,
                    sample_rate: 44_100,
                    channels: 2,
                    bitrate: None,
                },
                metadata_version: None,
                metadata_extracted_at: None,
                state: FileState::Available,
                created_at: Timestamp::from_millis(0),
                updated_at: Timestamp::from_millis(0),
            };
            media_files.save(&media_file).expect("catalogued");
            track_repo
                .save(&Track {
                    profile_id: profile.id,
                    media_file_id: media_file.id,
                    title: name.to_owned(),
                    artist_id: None,
                    album_id: None,
                    track_no: None,
                    disc_no: None,
                    year: None,
                    added_at: Timestamp::from_millis(0),
                    removed_at: None,
                })
                .expect("in the library");
            media_file.id
        })
        .collect();

    let engine = Arc::new(FakeEngine::default());
    let playback = PlaybackService::new(
        Arc::clone(&context),
        PlaybackPorts {
            engine: Arc::clone(&engine) as _,
            media_files: Arc::new(SqliteMediaFileRepository::new(db.pool().clone())),
            tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
            history: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    );

    Harness {
        context: Arc::clone(&context),
        pool: db.pool().clone(),
        history: SqliteHistoryRepository::new(db.pool().clone()),
        playback,
        engine,
        profiles,
        profile_id: profile.id,
        tracks,
        _db: db,
    }
}

impl Harness {
    /// Everything written down, newest first.
    fn listens(&self) -> Vec<cadenza_core::domain::stats::PlayEvent> {
        self.history
            .recent(self.profile_id, Timestamp::from_millis(0), 100)
            .expect("read")
    }

    /// Plays a track, hears `seconds` of it, and moves on.
    fn hear(&self, index: usize, seconds: u64) {
        self.playback
            .play_track(self.tracks[index], PlaySource::Library)
            .expect("played");
        // What the listener heard, told to the engine the way playing it would.
        AudioEnginePort::seek(self.engine.as_ref(), PlaybackPosition::from_secs(seconds))
            .expect("heard that much");
    }
}

#[test]
fn a_listen_is_written_down_when_the_next_one_starts() {
    let harness = harness();

    harness.hear(0, 120);
    assert!(harness.listens().is_empty(), "nothing is written mid-track");

    harness.hear(1, 5);
    let listens = harness.listens();
    assert_eq!(
        listens.len(),
        1,
        "the first one closed when the second began"
    );

    let first = &listens[0];
    assert_eq!(first.media_file_id, harness.tracks[0]);
    assert_eq!(first.played, DurationMs::from_secs(120));
    assert_eq!(first.duration, TRACK);
    assert_eq!(first.source, PlaySource::Library);
    assert_eq!(first.outcome, PlayOutcome::Completed, "two thirds of it");
    assert!(first.ended_at.is_some());
}

#[test]
fn how_a_listen_ended_is_classified_as_the_rules_say() {
    let harness = harness();

    // Five seconds of three minutes: a skip (2.6, "до 10 секунд").
    harness.hear(0, 5);
    harness.playback.stop().expect("stopped");
    assert_eq!(harness.listens()[0].outcome, PlayOutcome::Skipped);

    // A minute of three: neither a skip nor a completion.
    harness.hear(1, 60);
    harness.playback.stop().expect("stopped");
    assert_eq!(harness.listens()[0].outcome, PlayOutcome::Partial);
}

#[test]
fn a_join_counts_the_track_as_heard_to_its_end() {
    let harness = harness();
    harness.hear(0, 170);

    // The engine handed over by itself, so the outgoing track ran out. Its
    // position cannot be asked for any more — the engine reports the new one.
    harness
        .playback
        .adopt(harness.tracks[1], PlaySource::Library)
        .expect("adopted");

    let listens = harness.listens();
    assert_eq!(listens.len(), 1);
    assert_eq!(listens[0].played, TRACK, "heard to the end");
    assert_eq!(listens[0].outcome, PlayOutcome::Completed);
}

#[test]
fn nothing_is_written_for_a_listener_who_turned_history_off() {
    let harness = harness();
    harness
        .profiles
        .set_history_enabled(harness.profile_id, false)
        .expect("history off");

    harness.hear(0, 120);
    harness.playback.stop().expect("stopped");

    assert!(
        harness.listens().is_empty(),
        "history off means nothing stored — not a reduced record, not an anonymised one"
    );
}

#[test]
fn what_was_played_most_is_what_was_played_through() {
    let harness = harness();

    // The first track heard twice, in full; the second started four times and
    // abandoned each time.
    for _ in 0..2 {
        harness.hear(0, 175);
    }
    for _ in 0..4 {
        harness.hear(1, 3);
    }
    harness.playback.stop().expect("stopped");

    let top = harness
        .history
        .top_tracks(harness.profile_id, Timestamp::from_millis(0), 10)
        .expect("counted");

    assert_eq!(
        top,
        vec![(harness.tracks[0], 2)],
        "a track skipped four times is not a favourite"
    );
}

#[test]
fn history_older_than_the_window_is_cleared_and_the_rest_is_kept() {
    let harness = harness();
    harness.hear(0, 120);
    harness.playback.stop().expect("stopped");

    // The clock the listen was stamped by, not zero: the cutoff is measured
    // back from a moment, and a moment thirty years before the listen purges
    // nothing whatever the window is.
    let now = cadenza_testkit::test_clock::DEFAULT_START;
    let removed = harness
        .history
        .purge_before(harness.profile_id, cutoff(now, HISTORY_RETENTION_DAYS))
        .expect("purged");
    assert_eq!(removed, 0, "today's listen is not old");
    assert_eq!(harness.listens().len(), 1);

    // A month and a day on, the same listen has aged out.
    let later = Timestamp::from_millis(
        now.as_millis() + i64::from(HISTORY_RETENTION_DAYS + 1) * 24 * 60 * 60 * 1_000,
    );
    let removed = harness
        .history
        .purge_before(harness.profile_id, cutoff(later, HISTORY_RETENTION_DAYS))
        .expect("purged");
    assert_eq!(removed, 1);
    assert!(harness.listens().is_empty());
}

#[test]
fn switching_history_off_can_forget_what_was_already_written() {
    let harness = harness();
    harness.hear(0, 120);
    harness.playback.stop().expect("stopped");
    assert_eq!(harness.listens().len(), 1);

    let removed = harness
        .history
        .purge_all(harness.profile_id)
        .expect("forgotten");
    assert_eq!(removed, 1);
    assert!(harness.listens().is_empty());
}

#[test]
fn a_month_of_listening_adds_up() {
    use cadenza_core::application::services::{StatsPorts, StatsService};

    let harness = harness();

    // Two tracks played through, one of them twice; one abandoned early.
    harness.hear(0, 175);
    harness.hear(0, 175);
    harness.hear(1, 170);
    harness.hear(1, 4);
    harness.playback.stop().expect("stopped");

    let stats = StatsService::new(
        Arc::clone(&harness.context),
        StatsPorts {
            history: Arc::new(SqliteHistoryRepository::new(harness.pool.clone())),
            stats: Arc::new(SqliteHistoryRepository::new(harness.pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(harness.pool.clone())),
        },
    );

    let report = stats.report().expect("a report");
    assert!(report.keeping, "this profile is keeping history");
    assert_eq!(report.summary.started, 4);
    assert_eq!(report.summary.completed, 3);
    assert_eq!(report.summary.skipped, 1);
    assert_eq!(report.summary.tracks, 2, "two distinct files");
    assert!((report.summary.skip_rate() - 0.25).abs() < 1e-6);

    // 175 + 175 + 170 + 4 seconds.
    assert_eq!(report.summary.listened, DurationMs::from_secs(524));

    assert_eq!(
        report
            .top
            .first()
            .map(|(track, plays)| (track.title.as_str(), *plays)),
        Some(("one", 2)),
        "the one played through twice leads"
    );
}

#[test]
fn a_listener_who_keeps_no_history_is_told_so_rather_than_shown_zeroes() {
    use cadenza_core::application::services::{StatsPorts, StatsService};

    let harness = harness();
    harness.hear(0, 175);
    harness.playback.stop().expect("stopped");

    harness
        .profiles
        .set_history_enabled(harness.profile_id, false)
        .expect("history off");

    let stats = StatsService::new(
        Arc::clone(&harness.context),
        StatsPorts {
            history: Arc::new(SqliteHistoryRepository::new(harness.pool.clone())),
            stats: Arc::new(SqliteHistoryRepository::new(harness.pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(harness.pool.clone())),
        },
    );

    assert!(
        !stats.report().expect("a report").keeping,
        "an empty page and a page that is empty on purpose are different things"
    );

    // And turning it off can take what was already written with it.
    assert_eq!(stats.forget(harness.profile_id).expect("forgotten"), 1);
    assert!(harness.listens().is_empty());
}
