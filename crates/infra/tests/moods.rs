//! The moods that ship with Cadenza, read back from a migrated database.
//!
//! What is under test is not the SQL but the claim the SQL makes: that Workout
//! asks for fast music, that Sleep asks for slow, and that a listener starting
//! radio in a mood gets what the name promised.

use std::sync::Arc;

use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, MoodId};
use cadenza_core::domain::mood::{BUILTIN_MOOD_NAMES, FeatureBand, MoodPreset, MoodRules};
use cadenza_core::domain::playback::TransitionProfile;
use cadenza_core::domain::policies::radio_policy::{LibraryScale, mood_score};
use cadenza_core::domain::ports::repositories::MoodRepositoryPort;
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::track::TrackFeatures;
use cadenza_core::domain::value_objects::{Bpm, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteMoodRepository, SqliteProfileRepository, SqliteSettingsRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

struct Harness {
    moods: SqliteMoodRepository,
    profile: Profile,
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
    let profile = ProfileService::new(context)
        .create("Sasha")
        .expect("a profile");

    Harness {
        moods: SqliteMoodRepository::new(db.pool().clone()),
        profile,
        _db: db,
    }
}

/// A library with a shape: slow and quiet at one end, fast and loud at the
/// other, and the middle filled in.
///
/// A mood is now judged against the library it is choosing from , so a test of
/// what a mood means needs one. Two tracks on their own would put both of them
/// in the middle of their own distribution, which is true and useless.
fn library() -> Vec<TrackFeatures> {
    [
        (55.0, 0.10),
        (70.0, 0.20),
        (85.0, 0.30),
        (100.0, 0.45),
        (115.0, 0.55),
        (130.0, 0.70),
        (145.0, 0.80),
        (160.0, 0.90),
    ]
    .into_iter()
    .map(|(bpm, energy)| track(bpm, energy))
    .collect()
}

/// A track with a tempo and an intensity, and nothing else worth mentioning.
fn track(bpm: f32, energy: f32) -> TrackFeatures {
    TrackFeatures {
        media_file_id: MediaFileId::new(),
        bpm: Some(Bpm::new(bpm).expect("in range")),
        bpm_confidence: 1.0,
        key: None,
        energy,
        loudness: energy,
        spectral_centroid: 0.5,
        spectral_rolloff: 0.5,
        danceability: energy,
        valence: 0.6,
        tempo_stability: 1.0,
        dynamic_range: 0.5,
        extractor_version: "test".to_owned(),
        analyzed_at: Timestamp::from_millis(0),
    }
}

impl Harness {
    fn mood(&self, name: &str) -> MoodPreset {
        self.moods
            .list_for_profile(self.profile.id)
            .expect("moods")
            .into_iter()
            .find(|preset| preset.name == name)
            .unwrap_or_else(|| panic!("{name} is not among the built-in moods"))
    }
}

#[test]
fn every_mood_the_specification_names_is_there_and_asks_for_something() {
    let harness = harness();
    let moods = harness
        .moods
        .list_for_profile(harness.profile.id)
        .expect("moods");

    assert_eq!(moods.len(), BUILTIN_MOOD_NAMES.len());
    for name in BUILTIN_MOOD_NAMES {
        let mood = harness.mood(name);
        assert!(mood.is_builtin, "{name} should have shipped");
        assert!(!mood.is_editable(), "and should not be editable");
        assert!(
            !mood.rules.is_unconstrained(),
            "{name} asks for nothing, which makes it the same mood as every other"
        );
    }
}

#[test]
fn the_moods_mean_what_their_names_say() {
    let harness = harness();

    let library = library();
    let scale = LibraryScale::of(&library);
    let lullaby = library.first().expect("the slow end");
    let sprint = library.last().expect("the fast one");

    for (name, expected) in [("Workout", sprint), ("Sleep", lullaby)] {
        let mood = harness.mood(name);
        let wanted = mood_score(&mood.rules, Some(expected), &scale);
        let other = mood_score(
            &mood.rules,
            Some(if name == "Workout" { lullaby } else { sprint }),
            &scale,
        );

        assert!(
            wanted > 0.9,
            "{name} should love its own kind of track, scored {wanted}"
        );
        assert!(
            other < 0.1,
            "{name} should have no use for the other, scored {other}"
        );
    }
}

#[test]
fn a_quiet_mood_and_a_loud_one_disagree_about_the_same_track() {
    let harness = harness();
    let library = library();
    let scale = LibraryScale::of(&library);
    // 130 BPM and 0.70 energy - the loudest track that is still inside Party's
    // tempo. The fixture's actual last row is 160, which is a Workout track:
    // now that a band missed outright zeroes the mood, Party scores it 0 too
    // and the comparison stopped saying anything about energy.
    let banger = &library[5];

    let party = mood_score(&harness.mood("Party").rules, Some(banger), &scale);
    let focus = mood_score(&harness.mood("Focus").rules, Some(banger), &scale);

    assert!(
        party > focus + 0.5,
        "Party {party} should be far happier with it than Focus {focus}"
    );
}

#[test]
fn a_listener_can_keep_a_mood_of_their_own_but_not_edit_a_built_in() {
    let harness = harness();

    let mine = MoodPreset {
        id: MoodId::new(),
        profile_id: Some(harness.profile.id),
        name: "Late night".to_owned(),
        is_builtin: false,
        rules: MoodRules {
            bpm: Some(FeatureBand::new(70.0, 100.0, 15.0)),
            energy: Some(FeatureBand::new(0.1, 0.4, 0.2)),
            ..MoodRules::default()
        },
        genre_boost_json: None,
        ranking_weights_json: None,
        transition: TransitionProfile::Crossfade,
        created_at: Timestamp::from_millis(1),
        updated_at: Timestamp::from_millis(1),
    };
    harness.moods.save(&mine).expect("saved");

    let read_back = harness.mood("Late night");
    assert_eq!(read_back.rules, mine.rules, "the bands came back unchanged");
    assert!(read_back.is_editable());

    // The built-ins are what a listener gets back to when their own mood turns
    // out to be a bad idea, so nothing may write over one.
    let mut tampered = harness.mood("Sleep");
    tampered.name = "Sleep (mine)".to_owned();
    assert!(harness.moods.save(&tampered).is_err());
    assert!(harness.moods.delete(tampered.id).is_err());

    harness.moods.delete(mine.id).expect("deleted");
    assert_eq!(
        harness
            .moods
            .list_for_profile(harness.profile.id)
            .expect("moods")
            .len(),
        BUILTIN_MOOD_NAMES.len()
    );
}
