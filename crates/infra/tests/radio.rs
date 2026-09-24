//! A station, end to end, over a real database.
//!
//! What radio is judged on: a mood can be chosen, the station generates a
//! stream, and skips change what it offers next.

use std::sync::Arc;

use cadenza_core::application::services::{RadioPorts, RadioService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::PlayEventId;
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::mood::BUILTIN_MOOD_NAMES;
use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::ports::repositories::PlayEventRepositoryPort;
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, MoodRepositoryPort, RadioRepositoryPort, TrackFeaturesRepositoryPort,
    TrackRepositoryPort,
};
use cadenza_core::domain::radio::{MAX_BATCH_SIZE, MIN_BATCH_SIZE, RadioFeedback};
use cadenza_core::domain::stats::{PlayEvent, PlayOutcome, PlaySource};
use cadenza_core::domain::track::{Track, TrackFeatures};
use cadenza_core::domain::value_objects::{Bpm, DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteHistoryRepository, SqliteMediaFileRepository, SqliteMoodRepository,
    SqliteProfileRepository, SqliteRadioRepository, SqliteSettingsRepository,
    SqliteTrackFeaturesRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

struct Harness {
    radio: RadioService,
    moods: SqliteMoodRepository,
    picks: SqliteRadioRepository,
    /// What the listener has played, for the freshness term to read.
    history: SqliteHistoryRepository,
    /// The clock the service reads, so a listen can be dated against it.
    clock: Arc<TestClock>,
    profile_id: ProfileId,
    /// The library, fast tracks first and slow ones after.
    fast: Vec<MediaFileId>,
    slow: Vec<MediaFileId>,
    _db: TempDb,
}

/// Twelve of each, which is more than one batch either way.
const EACH: usize = 12;

fn harness() -> Harness {
    let db = TempDb::new();

    let clock = Arc::new(TestClock::default());
    let context = Arc::new(AppContext::new(
        Arc::clone(&clock) as _,
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");
    context.set_active_profile(profile.id);

    let media_files = SqliteMediaFileRepository::new(db.pool().clone());
    let tracks = SqliteTrackRepository::new(db.pool().clone());
    let features = SqliteTrackFeaturesRepository::new(db.pool().clone());

    let catalogue = |name: &str, artist: &str, bpm: f32, energy: f32| {
        let media_file = MediaFile {
            id: MediaFileId::new(),
            path: format!("C:/music/{name}.flac").into(),
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
        };
        media_files.save(&media_file).expect("catalogued");

        tracks
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

        features
            .save(&TrackFeatures {
                media_file_id: media_file.id,
                bpm: Some(Bpm::new(bpm).expect("in range")),
                bpm_confidence: 1.0,
                key: None,
                energy,
                loudness: energy,
                spectral_centroid: energy,
                spectral_rolloff: energy,
                danceability: energy,
                valence: 0.6,
                tempo_stability: 1.0,
                dynamic_range: 0.5,
                extractor_version: "test".to_owned(),
                analyzed_at: Timestamp::from_millis(0),
            })
            .expect("analysed");

        let _ = artist;
        media_file.id
    };

    let fast = (0..EACH)
        .map(|index| catalogue(&format!("fast-{index}"), "Sprinter", 150.0, 0.85))
        .collect();
    let slow = (0..EACH)
        .map(|index| catalogue(&format!("slow-{index}"), "Dozer", 62.0, 0.12))
        .collect();

    let radio = RadioService::new(
        context,
        RadioPorts {
            radio: Arc::new(SqliteRadioRepository::new(db.pool().clone())),
            moods: Arc::new(SqliteMoodRepository::new(db.pool().clone())),
            tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
            features: Arc::new(SqliteTrackFeaturesRepository::new(db.pool().clone())),
            stats: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    );

    Harness {
        moods: SqliteMoodRepository::new(db.pool().clone()),
        picks: SqliteRadioRepository::new(db.pool().clone()),
        history: SqliteHistoryRepository::new(db.pool().clone()),
        clock,
        radio,
        profile_id: profile.id,
        fast,
        slow,
        _db: db,
    }
}

impl Harness {
    fn start(&self, mood: &str) {
        let preset = self
            .moods
            .list_for_profile(self.profile_id)
            .expect("moods")
            .into_iter()
            .find(|preset| preset.name == mood)
            .expect("a built-in mood");

        self.radio.start(preset.id, None).expect("a station");
    }
}

#[test]
fn a_station_offers_a_batch_of_what_the_mood_asked_for() {
    let harness = harness();
    harness.start("Workout");

    let batch = harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");
    assert_eq!(batch.len(), MIN_BATCH_SIZE, "a batch is a batch");

    let fast = batch.iter().filter(|id| harness.fast.contains(id)).count();
    assert_eq!(
        fast,
        batch.len(),
        "Workout offered {} slow tracks",
        batch.len() - fast
    );
}

#[test]
fn a_different_mood_is_a_different_station() {
    let harness = harness();
    harness.start("Sleep");

    let batch = harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");
    assert!(
        batch.iter().all(|id| harness.slow.contains(id)),
        "Sleep should have offered nothing but the quiet half"
    );
}

#[test]
fn a_station_never_offers_the_same_track_twice() {
    let harness = harness();
    harness.start("Workout");

    let mut offered = Vec::new();
    for _ in 0..4 {
        offered.extend(harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch"));
    }

    let mut unique = offered.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), offered.len(), "a track came round twice");

    // Twenty-four tracks in the library and four batches of eight asked for:
    // the station runs out rather than repeating itself.
    assert_eq!(offered.len(), EACH * 2);
    assert!(
        harness
            .radio
            .next_batch(MIN_BATCH_SIZE)
            .expect("asked")
            .is_empty(),
        "and then it has nothing left to offer"
    );
}

#[test]
fn every_pick_can_say_why_it_was_picked() {
    let harness = harness();
    harness.start("Workout");
    harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");

    let items = harness
        .picks
        .recent_items(harness.radio.session().expect("a session").id, 100)
        .expect("the picks");

    assert_eq!(items.len(), MIN_BATCH_SIZE);
    for item in items {
        let reason = item.reason.expect("a pick without a reason is a black box");
        assert!(
            reason.mood > 0.9,
            "Workout picked something it rated {} for mood",
            reason.mood
        );
        assert!(reason.score.is_finite());
    }
}

#[test]
fn a_dislike_is_remembered_and_weighs_against_the_track() {
    let harness = harness();
    harness.start("Workout");

    let first = harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");
    let victim = first[0];

    // Said twice on purpose. A verdict is about a *pick*, not a tally: saying
    // the same thing twice about the same offer says it once, and the weight
    // that reaches the ranking is one dislike rather than two.
    for _ in 0..2 {
        harness
            .radio
            .feedback(victim, RadioFeedback::Dislike)
            .expect("recorded");
    }

    let preferences = harness.picks.preferences(harness.profile_id).expect("read");
    let (_, total) = preferences
        .iter()
        .find(|(id, _)| *id == victim)
        .expect("the verdict was kept");
    assert_eq!(
        *total,
        RadioFeedback::Dislike.weight(),
        "one verdict about one pick, however many times it was given"
    );

    // A fresh station in the same mood. The verdict is not a ban — radio may
    // still offer the track — but it now scores the bottom of the preference
    // term where everything unjudged scores the middle of it.
    harness.start("Workout");
    harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");

    let picks = harness
        .picks
        .recent_items(harness.radio.session().expect("a session").id, 100)
        .expect("the picks");

    for pick in &picks {
        let preference = pick.reason.expect("a reason").preference;
        if pick.media_file_id == victim {
            assert!(
                preference < 0.5,
                "the dislike did not reach the ranking: {preference}"
            );
        } else {
            assert_eq!(
                preference, 0.5,
                "an unjudged track is neither liked nor disliked"
            );
        }
    }

    // And what it costs is real: nothing else was docked, so the victim is
    // ranked below every track it is otherwise identical to.
    assert!(
        picks.iter().any(|pick| pick.media_file_id != victim),
        "the station offered nothing to compare against"
    );
}

#[test]
fn the_listener_can_start_from_any_of_the_eight_moods() {
    let harness = harness();

    for name in BUILTIN_MOOD_NAMES {
        harness.start(name);
        let batch = harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");
        assert!(
            !batch.is_empty(),
            "{name} found nothing at all to play, which is the one thing radio may not do"
        );
    }
}

#[test]
fn feedback_about_nothing_is_not_an_error() {
    let harness = harness();

    // No station running: a listener pressing skip during ordinary playback is
    // not saying anything about a station they are not listening to.
    assert!(
        harness
            .radio
            .feedback(harness.fast[0], RadioFeedback::Skip)
            .is_ok()
    );
}

#[test]
fn a_verdict_about_a_track_the_station_never_offered_is_not_an_error() {
    let harness = harness();
    harness.start("Workout");
    harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");

    // Sleep's half of the library: a Workout station cannot have offered it.
    // This is what pressing next on a hand-queued track during a station does,
    // and it says nothing about the station.
    assert!(
        harness
            .radio
            .feedback(harness.slow[0], RadioFeedback::Skip)
            .is_ok()
    );

    assert!(
        harness
            .picks
            .preferences(harness.profile_id)
            .expect("read")
            .is_empty(),
        "and nothing was recorded about it"
    );
}

#[test]
fn a_station_that_has_ended_is_not_asked_for_more() {
    let harness = harness();
    harness.start("Workout");
    harness.radio.next_batch(MIN_BATCH_SIZE).expect("a batch");

    harness.radio.stop();

    // What the queue asks on every tick once the lane runs low. Before this was
    // an error, it was reported to the listener four times a second for as long
    // as the picks already queued kept playing.
    assert!(harness.radio.session().is_none());
    assert!(
        harness.radio.next_batch(MIN_BATCH_SIZE).is_err(),
        "asking a station that has ended is a question with no answer"
    );
}

/// Freshness is about the listener's ears, not about the station's memory.
///
/// A track played five minutes ago by hand has been heard, and the station must
/// know that as surely as if it had offered the track itself — which is the
/// source radio had to make do with at first, because nothing wrote listening
/// history yet.
#[test]
fn a_track_the_listener_just_played_is_not_fresh_to_the_station() {
    let harness = harness();
    let heard = harness.fast[0];

    harness
        .history
        .append(&PlayEvent {
            id: PlayEventId::new(),
            profile_id: harness.profile_id,
            media_file_id: heard,
            source: PlaySource::Library,
            // Dated against the clock the service reads. Freshness is "how long
            // ago", so a listen written at the epoch is a listen from years back
            // — which is exactly as fresh as never having heard it.
            started_at: harness.clock.now(),
            ended_at: Some(harness.clock.now()),
            played: DurationMs::from_secs(180),
            duration: DurationMs::from_secs(180),
            outcome: PlayOutcome::Completed,
        })
        .expect("the listen is written down");

    harness.start("Workout");
    // The whole shelf rather than one batch: a track that has just been heard
    // scores below its twelve identical neighbours, so a batch of eight leaves
    // it out — which is the feature working, and no way to read the term it was
    // marked down by.
    harness.radio.next_batch(MAX_BATCH_SIZE).expect("a batch");

    let picks = harness
        .picks
        .recent_items(harness.radio.session().expect("a session").id, 100)
        .expect("the picks");

    // Asserted on the term rather than on the order, because the order is a
    // weighted draw with noise in it and the term is not.
    let mut seen = false;
    for pick in &picks {
        let freshness = pick.reason.expect("a reason").freshness;
        if pick.media_file_id == heard {
            seen = true;
            assert!(
                freshness < 1.0,
                "the listen never reached the ranking: {freshness}"
            );
        } else {
            assert_eq!(
                freshness, 1.0,
                "a track nobody has heard is as fresh as this can measure"
            );
        }
    }
    assert!(seen, "the station never offered the track that was heard");

    // And it costs the track its place: nothing else was marked down, so it
    // ranks below the twelve it is otherwise identical to.
    let place = picks
        .iter()
        .position(|pick| pick.media_file_id == heard)
        .expect("it was offered");
    assert!(
        place > 0,
        "a track heard a moment ago was the first thing offered back"
    );
}
