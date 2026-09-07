//! The level a listener set is the level they come back to.
//!
//! A real database and a fake engine. What is under test is the half of it no
//! amount of audio work provides: that turning the volume down is a decision
//! and not a gesture, and a decision outlives the run it was made in.

use std::sync::Arc;

use cadenza_core::application::services::{PlaybackPorts, PlaybackService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::value_objects::Volume;
use cadenza_infra::db::repositories::{
    SqliteHistoryRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

mod fake_engine;
use fake_engine::FakeEngine;

/// One run of the application, over a database that outlives it.
fn run(db: &TempDb, context: &Arc<AppContext>) -> PlaybackService {
    PlaybackService::new(
        Arc::clone(context),
        PlaybackPorts {
            engine: Arc::new(FakeEngine::default()) as _,
            media_files: Arc::new(SqliteMediaFileRepository::new(db.pool().clone())),
            tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
            history: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    )
}

fn context(db: &TempDb) -> Arc<AppContext> {
    Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ))
}

#[test]
fn the_level_survives_the_run_it_was_set_in() {
    let db = TempDb::new();
    let context = context(&db);
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");
    context.set_active_profile(profile.id);

    let quiet = Volume::new(0.4).expect("in range");
    run(&db, &context).set_volume(quiet).expect("turned down");

    // The next morning: a new service over the same database, which is what a
    // second run is.
    let next = run(&db, &context);
    assert_eq!(
        next.view().volume,
        Volume::FULL,
        "nothing is known until it is asked for"
    );

    next.restore_volume().expect("restored");
    assert_eq!(next.view().volume, quiet);
}

#[test]
fn a_profile_that_has_never_touched_it_hears_something() {
    let db = TempDb::new();
    let context = context(&db);
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");
    context.set_active_profile(profile.id);

    let playback = run(&db, &context);
    playback.restore_volume().expect("restored");

    // The first run of a music player should make a sound.
    assert_eq!(playback.view().volume, Volume::FULL);
}

#[test]
fn two_listeners_do_not_share_what_loud_enough_means() {
    let db = TempDb::new();
    let context = context(&db);
    let profiles = ProfileService::new(Arc::clone(&context));
    let sasha = profiles.create("Sasha").expect("a profile");
    let other = profiles.create("Someone Else").expect("a profile");

    context.set_active_profile(sasha.id);
    let quiet = Volume::new(0.2).expect("in range");
    run(&db, &context).set_volume(quiet).expect("turned down");

    context.set_active_profile(other.id);
    let playback = run(&db, &context);
    playback.restore_volume().expect("restored");
    assert_eq!(
        playback.view().volume,
        Volume::FULL,
        "somebody else's evening is not this listener's"
    );

    context.set_active_profile(sasha.id);
    let playback = run(&db, &context);
    playback.restore_volume().expect("restored");
    assert_eq!(playback.view().volume, quiet);
}
