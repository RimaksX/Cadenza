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
use cadenza_core::domain::ids::ProfileId;
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::settings::CrossfadeDuration;
use cadenza_core::domain::value_objects::{GainDb, PlaybackPosition, Volume};
use cadenza_infra::db::repositories::{
    SqliteEqPresetRepository, SqliteProfileRepository, SqliteSettingsRepository,
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
