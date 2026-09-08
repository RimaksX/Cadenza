//! The equaliser's settings: what reaches the filters, and what survives a
//! restart.
//!
//! A real database and a fake set of filters. What is under test is that the
//! sound a listener left behind is the sound they come back to — which is the
//! half of PROJECT_MASTER 2.8 that no amount of DSP can provide.

use std::path::Path;
use std::sync::{Arc, Mutex};

use cadenza_core::Result;
use cadenza_core::application::services::{EqPorts, EqService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::eq::{EqBand, EqMode, EqSetting, SimpleEq};
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::repositories::MediaFileRepositoryPort;
use cadenza_core::domain::settings::CrossfadeDuration;
use cadenza_core::domain::value_objects::{DurationMs, GainDb, PlaybackPosition, Volume};
use cadenza_infra::db::repositories::{
    SqliteEqPresetRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackEqRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

/// Filters that remember what they were last told.
#[derive(Default)]
struct FakeEngine {
    applied: Mutex<Option<EqSetting>>,
}

impl FakeEngine {
    fn applied(&self) -> EqSetting {
        self.applied
            .lock()
            .expect("not poisoned")
            .clone()
            .expect("nothing was ever applied")
    }
}

impl AudioEnginePort for FakeEngine {
    fn load(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
    fn preload_next(&self, _path: &Path, _transition: TransitionProfile) -> Result<()> {
        Ok(())
    }
    fn armed(&self) -> bool {
        false
    }
    fn advances(&self) -> u64 {
        0
    }
    fn play(&self) -> Result<()> {
        Ok(())
    }
    fn pause(&self) -> Result<()> {
        Ok(())
    }
    fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn seek(&self, _position: PlaybackPosition) -> Result<()> {
        Ok(())
    }
    fn set_volume(&self, _volume: Volume) -> Result<()> {
        Ok(())
    }
    fn set_crossfade(&self, _duration: CrossfadeDuration) -> Result<()> {
        Ok(())
    }
    fn set_eq(&self, setting: &EqSetting) -> Result<()> {
        *self.applied.lock().expect("not poisoned") = Some(setting.clone());
        Ok(())
    }
    fn position(&self) -> PlaybackPosition {
        PlaybackPosition::START
    }
    fn state(&self) -> PlaybackState {
        PlaybackState::Stopped
    }
}

struct Harness {
    eq: EqService,
    engine: Arc<FakeEngine>,
    profile_id: ProfileId,
    /// Declared last: the fixture cannot delete its directory while anything
    /// above it still holds a connection into it.
    db: TempDb,
}

fn harness() -> Harness {
    let db = TempDb::new();
    let profile_id = {
        let context = context(&db);
        let profile = ProfileService::new(Arc::clone(&context))
            .create("Sasha")
            .expect("a profile");
        context.set_active_profile(profile.id);
        profile.id
    };

    let (eq, engine) = service(&db, profile_id);
    Harness {
        eq,
        engine,
        profile_id,
        db,
    }
}

impl Harness {
    /// A file in the catalogue, which is what a choice can be recorded against.
    fn track(&self, name: &str) -> MediaFileId {
        let file = MediaFile {
            id: MediaFileId::new(),
            path: std::path::PathBuf::from(format!("C:/music/{name}.flac")),
            file_hash: None,
            file_size: 1_024,
            file_mtime: cadenza_core::domain::value_objects::Timestamp::from_millis(0),
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
            created_at: cadenza_core::domain::value_objects::Timestamp::from_millis(0),
            updated_at: cadenza_core::domain::value_objects::Timestamp::from_millis(0),
        };

        SqliteMediaFileRepository::new(self.db.pool().clone())
            .save(&file)
            .expect("catalogued");
        file.id
    }

    /// The preset with this name, as the listener would press it.
    fn preset(&self, name: &str) -> cadenza_core::domain::ids::EqPresetId {
        self.eq
            .list()
            .expect("presets")
            .into_iter()
            .find(|preset| preset.name == name)
            .unwrap_or_else(|| panic!("{name} ships"))
            .id
    }
}

fn context(db: &TempDb) -> Arc<AppContext> {
    Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ))
}

/// The equaliser as one run of the application builds it.
fn service(db: &TempDb, profile_id: ProfileId) -> (EqService, Arc<FakeEngine>) {
    let context = context(db);
    context.set_active_profile(profile_id);

    let engine = Arc::new(FakeEngine::default());
    let eq = EqService::new(
        context,
        EqPorts {
            presets: Arc::new(SqliteEqPresetRepository::new(db.pool().clone())),
            choices: Arc::new(SqliteTrackEqRepository::new(db.pool().clone())),
            engine: Arc::clone(&engine) as Arc<dyn AudioEnginePort>,
        },
    );
    (eq, engine)
}

#[test]
fn a_fresh_profile_hears_nothing_added() {
    let harness = harness();
    let setting = harness.eq.current().expect("a setting");

    assert!(setting.is_flat());
    assert!(
        harness.engine.applied().is_flat(),
        "and the filters were told so, because they have no database to ask"
    );
}

#[test]
fn choosing_a_preset_leaves_the_listener_in_the_mode_they_are_in() {
    let harness = harness();
    let presets = harness.eq.list().expect("the built-ins");
    let rock = presets
        .iter()
        .find(|preset| preset.name == "Rock")
        .expect("Rock ships");

    // In the simple mode, where a fresh profile starts: the three controls
    // move and nothing switches underneath the listener.
    harness.eq.apply_preset(rock.id).expect("applied");
    let applied = harness.engine.applied();
    assert_eq!(applied.mode, EqMode::Simple);
    assert_eq!(applied.simple, rock.simple, "the three controls moved");
    assert!(
        !applied.simple.is_flat(),
        "and a preset that says something says it here too"
    );

    // And in the advanced mode, the same preset moves the eight faders.
    harness.eq.set_mode(EqMode::Advanced).expect("switched");
    harness.eq.apply_preset(rock.id).expect("applied");
    let applied = harness.engine.applied();
    assert_eq!(applied.mode, EqMode::Advanced);
    assert_eq!(
        applied.advanced, rock.advanced,
        "every band, where the preset put it"
    );
}

#[test]
fn the_sound_a_listener_leaves_is_the_sound_they_come_back_to() {
    let harness = harness();

    harness
        .eq
        .set_band(
            2,
            EqBand::new(440, 2.5, GainDb::new(-7.5).expect("in range")).expect("in range"),
        )
        .expect("set");

    // A second service over the same database is what the next run of the
    // application is.
    let (restarted, engine) = service(&harness.db, harness.profile_id);
    let setting = restarted.current().expect("a setting");

    assert_eq!(setting.mode, EqMode::Advanced);
    assert_eq!(setting.advanced[2].frequency_hz(), 440);
    assert_eq!(setting.advanced[2].q(), 2.5);
    assert_eq!(setting.advanced[2].gain().as_db(), -7.5);
    assert_eq!(
        engine.applied(),
        setting,
        "and reading it is what puts it back on the filters"
    );
}

#[test]
fn the_three_controls_are_remembered_separately_from_the_bells() {
    let harness = harness();

    harness
        .eq
        .set_band(
            0,
            EqBand::new(60, 1.0, GainDb::new(9.0).expect("in range")).expect("in range"),
        )
        .expect("set");
    harness
        .eq
        .set_simple(SimpleEq {
            bass: GainDb::new(4.0).expect("in range"),
            ..SimpleEq::FLAT
        })
        .expect("set");

    assert_eq!(harness.engine.applied().mode, EqMode::Simple);

    // Back to the bells: what was set there is still there.
    harness.eq.set_mode(EqMode::Advanced).expect("switched");
    let applied = harness.engine.applied();
    assert_eq!(applied.advanced[0].gain().as_db(), 9.0);
    assert_eq!(
        applied.simple.bass.as_db(),
        4.0,
        "and the tone controls kept their own places"
    );
}

#[test]
fn a_setting_can_be_saved_under_a_name_and_chosen_again() {
    let harness = harness();
    harness
        .eq
        .set_band(
            7,
            EqBand::new(12_000, 3.0, GainDb::new(5.0).expect("in range")).expect("in range"),
        )
        .expect("set");

    let mine = harness.eq.save_as("  Late night  ").expect("saved");
    assert_eq!(mine.name, "Late night", "the name is trimmed");

    harness.eq.reset().expect("flat again");
    assert!(harness.engine.applied().is_flat());

    harness.eq.apply_preset(mine.id).expect("applied");
    let applied = harness.engine.applied();
    assert_eq!(applied.advanced[7].frequency_hz(), 12_000);
    assert_eq!(applied.advanced[7].gain().as_db(), 5.0);

    assert!(
        harness.eq.save_as("   ").is_err(),
        "a preset needs a name to be found by"
    );
}

#[test]
fn a_preset_belonging_to_somebody_else_is_not_found() {
    let harness = harness();
    let mine = harness.eq.save_as("Mine").expect("saved");

    // Another listener, on the same machine.
    let context = context(&harness.db);
    let other = ProfileService::new(Arc::clone(&context))
        .create("Kim")
        .expect("a profile");
    let (theirs, _) = service(&harness.db, other.id);

    assert!(
        theirs.apply_preset(mine.id).is_err(),
        "one profile's sound is not another's to choose"
    );
    assert_eq!(
        theirs.list().expect("listed").len(),
        9,
        "and they see only what shipped"
    );
}

#[test]
fn a_saved_sound_can_be_renamed_and_thrown_away() {
    let harness = harness();
    harness
        .eq
        .set_simple(SimpleEq {
            bass: GainDb::new(6.0).expect("in range"),
            ..SimpleEq::FLAT
        })
        .expect("set");

    let mine = harness.eq.save_as("Late night").expect("saved");
    harness
        .eq
        .rename(mine.id, "  Very late  ")
        .expect("renamed");

    let listed = harness.eq.list().expect("listed");
    let renamed = listed
        .iter()
        .find(|preset| preset.id == mine.id)
        .expect("still there");
    assert_eq!(renamed.name, "Very late", "and the name is trimmed");
    assert_eq!(
        renamed.simple.bass.as_db(),
        6.0,
        "a rename changes the name and nothing else"
    );

    harness.eq.delete(mine.id).expect("deleted");
    assert_eq!(harness.eq.list().expect("listed").len(), 9);
}

#[test]
fn a_name_already_taken_is_refused_before_the_database_refuses_it() {
    let harness = harness();
    harness.eq.save_as("Late night").expect("saved");

    let refused = harness.eq.save_as("late NIGHT").expect_err("taken");
    assert!(
        refused.to_string().contains("already a sound called"),
        "the refusal has to say what is wrong: {refused}"
    );

    // A built-in's name is taken too, and by something nobody can rename.
    assert!(harness.eq.save_as("Rock").is_err());

    // Renaming something to what it is already called is not a clash with
    // itself.
    let mine = harness
        .eq
        .list()
        .expect("listed")
        .into_iter()
        .find(|preset| preset.name == "Late night")
        .expect("saved above");
    assert!(harness.eq.rename(mine.id, "Late night").is_ok());
}

#[test]
fn the_nine_that_shipped_cannot_be_renamed_or_thrown_away() {
    let harness = harness();
    let rock = harness
        .eq
        .list()
        .expect("listed")
        .into_iter()
        .find(|preset| preset.name == "Rock")
        .expect("Rock ships");

    assert!(harness.eq.rename(rock.id, "Not Rock").is_err());
    assert!(harness.eq.delete(rock.id).is_err());
    assert_eq!(harness.eq.list().expect("listed").len(), 9);
}

#[test]
fn a_preset_chosen_while_a_track_plays_belongs_to_that_track() {
    let harness = harness();
    let downpour = harness.track("downpour");
    let deadlock = harness.track("deadlock");
    let bass_boost = harness.preset("Bass Boost");

    harness.eq.follow(downpour).expect("the track starts");
    harness.eq.apply_preset(bass_boost).expect("chosen");
    let chosen = harness.engine.applied();
    assert!(!chosen.is_flat(), "a boost is not nothing");

    // The next track is not the last track. Whatever was set for one record
    // does not follow the listener into the next one.
    harness.eq.follow(deadlock).expect("the next track starts");
    assert!(
        harness.engine.applied().is_flat(),
        "a track nobody chose for plays as it was recorded"
    );

    // And coming back to it is coming back to the sound it was given.
    harness.eq.follow(downpour).expect("round again");
    assert_eq!(harness.engine.applied(), chosen);
}

#[test]
fn a_choice_survives_a_restart() {
    let harness = harness();
    let downpour = harness.track("downpour");
    harness.eq.follow(downpour).expect("the track starts");
    harness
        .eq
        .apply_preset(harness.preset("Rock"))
        .expect("chosen");
    let chosen = harness.engine.applied();

    // The same database, opened again the way the next run opens it.
    let (eq, engine) = service(&harness.db, harness.profile_id);
    eq.follow(downpour).expect("the track starts again");
    assert_eq!(engine.applied(), chosen);
}

#[test]
fn resetting_forgets_what_the_track_was_chosen_to_sound_like() {
    let harness = harness();
    let downpour = harness.track("downpour");

    harness.eq.follow(downpour).expect("the track starts");
    harness
        .eq
        .apply_preset(harness.preset("Treble Boost"))
        .expect("chosen");
    harness.eq.reset().expect("reset");

    // Not merely flat now — flat the next time as well, which is the half a
    // reset that only cleared the filters would have got wrong.
    harness.eq.follow(downpour).expect("round again");
    assert!(harness.engine.applied().is_flat());
}

#[test]
fn a_preset_chosen_with_nothing_playing_is_just_the_sound() {
    let harness = harness();
    let downpour = harness.track("downpour");

    // Nobody has pressed play, so there is nothing to attach the choice to.
    harness
        .eq
        .apply_preset(harness.preset("Classical"))
        .expect("chosen");
    assert!(!harness.engine.applied().is_flat());

    // And the first track to start is a track nobody chose for.
    harness.eq.follow(downpour).expect("the track starts");
    assert!(harness.engine.applied().is_flat());
}
