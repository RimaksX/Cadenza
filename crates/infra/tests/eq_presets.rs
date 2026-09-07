//! The presets that ship with the application, and the ones a listener saves.
//!
//! A real database, because what is under test is largely the migration: nine
//! rows of hand-written JSON that have to come back as bands a filter can be
//! built from.

use std::sync::Arc;

use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::eq::{BUILTIN_PRESET_NAMES, EqBand, EqMode, EqPreset, SimpleEq};
use cadenza_core::domain::ids::{EqPresetId, ProfileId};
use cadenza_core::domain::policies::eq_policy::{
    ADVANCED_BAND_COUNT, default_advanced_bands, validate_advanced_bands,
};
use cadenza_core::domain::ports::repositories::EqPresetRepositoryPort;
use cadenza_core::domain::value_objects::{GainDb, Timestamp};
use cadenza_infra::db::repositories::SqliteSettingsRepository;
use cadenza_infra::db::repositories::{SqliteEqPresetRepository, SqliteProfileRepository};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

/// A profile and the preset store, wired as the application wires them.
struct Harness {
    presets: SqliteEqPresetRepository,
    profile_id: ProfileId,
    /// Declared last: the fixture cannot delete its directory while anything
    /// above it still holds a connection into it.
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

    Harness {
        presets: SqliteEqPresetRepository::new(db.pool().clone()),
        profile_id: profile.id,
        db,
    }
}

#[test]
fn every_preset_the_specification_names_is_there_and_can_be_built_into_filters() {
    let harness = harness();
    let all = harness
        .presets
        .list_for_profile(harness.profile_id)
        .expect("the built-ins");

    let names: Vec<&str> = all.iter().map(|preset| preset.name.as_str()).collect();
    for expected in BUILTIN_PRESET_NAMES {
        assert!(names.contains(&expected), "{expected} is missing");
    }
    assert_eq!(all.len(), BUILTIN_PRESET_NAMES.len(), "and nothing else");

    for preset in &all {
        assert!(preset.is_builtin);
        assert!(preset.profile_id.is_none(), "a built-in belongs to nobody");

        if preset.mode == EqMode::Advanced {
            assert_eq!(preset.advanced.len(), ADVANCED_BAND_COUNT);
            validate_advanced_bands(&preset.advanced).unwrap_or_else(|err| {
                panic!("{} has a band a filter cannot use: {err}", preset.name)
            });
        }
    }
}

#[test]
fn flat_is_the_one_that_does_nothing() {
    let harness = harness();
    let all = harness
        .presets
        .list_for_profile(harness.profile_id)
        .expect("the built-ins");

    let flat = all
        .iter()
        .find(|preset| preset.name == "Standard")
        .expect("Standard ships");

    assert_eq!(flat.mode, EqMode::Simple);
    assert!(flat.simple.is_flat());
}

#[test]
fn a_preset_of_the_listeners_own_comes_back_as_it_was_saved() {
    let harness = harness();

    let mut bands = default_advanced_bands();
    bands[2] = EqBand::new(440, 2.5, GainDb::new(-7.5).expect("in range")).expect("in range");

    let mine = EqPreset {
        id: EqPresetId::new(),
        profile_id: Some(harness.profile_id),
        name: "Late night".to_owned(),
        is_builtin: false,
        mode: EqMode::Advanced,
        simple: SimpleEq::FLAT,
        advanced: bands.clone(),
        created_at: Timestamp::from_millis(1),
        updated_at: Timestamp::from_millis(1),
    };
    harness.presets.save(&mine).expect("saved");

    let read = harness
        .presets
        .get(mine.id)
        .expect("read")
        .expect("it is there");

    assert_eq!(read.name, "Late night");
    assert_eq!(read.advanced, bands, "every band, to the decibel");
    assert_eq!(read.advanced[2].frequency_hz(), 440);
    assert_eq!(read.advanced[2].q(), 2.5);

    // And it is listed beside the built-ins, after them.
    let all = harness
        .presets
        .list_for_profile(harness.profile_id)
        .expect("listed");
    assert_eq!(all.len(), BUILTIN_PRESET_NAMES.len() + 1);
    assert_eq!(
        all.last().expect("a last one").name,
        "Late night",
        "the listener's own come after what shipped"
    );
}

#[test]
fn another_profile_never_sees_it() {
    let harness = harness();
    let mine = EqPreset {
        id: EqPresetId::new(),
        profile_id: Some(harness.profile_id),
        name: "Mine".to_owned(),
        is_builtin: false,
        mode: EqMode::Simple,
        simple: SimpleEq::FLAT,
        advanced: default_advanced_bands(),
        created_at: Timestamp::from_millis(1),
        updated_at: Timestamp::from_millis(1),
    };
    harness.presets.save(&mine).expect("saved");

    let someone_else = harness
        .presets
        .list_for_profile(ProfileId::new())
        .expect("listed");

    assert_eq!(
        someone_else.len(),
        BUILTIN_PRESET_NAMES.len(),
        "the built-ins are shared and nothing else is"
    );
}

#[test]
fn a_built_in_cannot_be_edited_or_deleted() {
    let harness = harness();
    let all = harness
        .presets
        .list_for_profile(harness.profile_id)
        .expect("the built-ins");
    let rock = all
        .iter()
        .find(|preset| preset.name == "Rock")
        .expect("Rock ships");

    let mut renamed = rock.clone();
    renamed.name = "Not Rock".to_owned();

    assert!(harness.presets.save(&renamed).is_err());
    assert!(harness.presets.delete(rock.id).is_err());

    let still_there = harness
        .presets
        .get(rock.id)
        .expect("read")
        .expect("it is there");
    assert_eq!(still_there.name, "Rock");
}

#[test]
fn deleting_takes_only_the_listeners_own() {
    let harness = harness();
    let mine = EqPreset {
        id: EqPresetId::new(),
        profile_id: Some(harness.profile_id),
        name: "Passing".to_owned(),
        is_builtin: false,
        mode: EqMode::Simple,
        simple: SimpleEq::FLAT,
        advanced: default_advanced_bands(),
        created_at: Timestamp::from_millis(1),
        updated_at: Timestamp::from_millis(1),
    };
    harness.presets.save(&mine).expect("saved");
    harness.presets.delete(mine.id).expect("deleted");

    assert!(harness.presets.get(mine.id).expect("read").is_none());
    assert!(
        harness.presets.delete(mine.id).is_err(),
        "deleting what is not there says so rather than passing quietly"
    );
    let _ = &harness.db;
}
