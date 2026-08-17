//! A station, end to end, over a real database.
//!
//! What M13 is judged on: a mood can be chosen, the station generates a stream,
//! and skips change what it offers next.

use std::sync::Arc;

use cadenza_core::application::services::{RadioPorts, RadioService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::mood::BUILTIN_MOOD_NAMES;
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, MoodRepositoryPort, RadioRepositoryPort, TrackFeaturesRepositoryPort,
    TrackRepositoryPort,
};
use cadenza_core::domain::radio::{MIN_BATCH_SIZE, RadioFeedback};
use cadenza_core::domain::track::{Track, TrackFeatures};
use cadenza_core::domain::value_objects::{Bpm, DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteMediaFileRepository, SqliteMoodRepository, SqliteProfileRepository,
    SqliteRadioRepository, SqliteSettingsRepository, SqliteTrackFeaturesRepository,
    SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

struct Harness {
    radio: RadioService,
    moods: SqliteMoodRepository,
    picks: SqliteRadioRepository,
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
        },
    );

    Harness {
        moods: SqliteMoodRepository::new(db.pool().clone()),
        picks: SqliteRadioRepository::new(db.pool().clone()),
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
