//! The join, through the real engine and a real sound card.
//!
//! Everything else about transitions is tested on the decode thread with no
//! device at all, which is where the arithmetic lives. This is the one thing
//! that cannot be: that the three threads agree — the decoder queues a join,
//! the callback crosses it, and the control side hears about it — on hardware
//! whose buffer size and sample rate nobody chose.
//!
//! Ignored by default. A machine without an output device is not a machine with
//! a failing test, and a build server has no speakers. Run it by hand:
//!
//! ```text
//! cargo test -p cadenza-infra --test audio_join -- --ignored
//! ```

use std::thread;
use std::time::{Duration, Instant};

use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::value_objects::Volume;
use cadenza_infra::audio::CpalAudioEngine;
use cadenza_testkit::TempDir;
use cadenza_testkit::audio_fixtures::write_wav;

/// Quiet, but not silent. Muted output never consumes the ring — that is what
/// pause is — so a muted engine would sit at the start of the first track
/// forever and the join would never arrive.
fn quietly() -> Volume {
    Volume::new(0.02).expect("in range")
}

/// Waits for the engine to say it has handed over.
fn wait_for_the_join(engine: &CpalAudioEngine) -> bool {
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if engine.advances() > 0 {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
#[ignore = "needs a real output device"]
fn the_engine_joins_two_files_without_stopping_between_them() {
    let directory = TempDir::new("engine-join");
    let first = write_wav(directory.path(), "first.wav", 1, 12_000);
    let second = write_wav(directory.path(), "second.wav", 1, -12_000);

    let engine = CpalAudioEngine::new().expect("an output device");
    engine.set_volume(quietly()).expect("quiet");
    engine.load(&first).expect("loaded");
    engine
        .preload_next(&second, TransitionProfile::Gapless)
        .expect("armed");
    assert!(engine.armed(), "the next track is open and waiting");

    engine.play().expect("playing");

    assert!(wait_for_the_join(&engine), "the join never arrived");
    assert_eq!(engine.advances(), 1, "and it happened once");
    assert!(!engine.armed(), "what was armed is now what is playing");
    assert_ne!(
        engine.state(),
        PlaybackState::Stopped,
        "nothing stopped to make the handover"
    );
    assert_eq!(engine.underruns(), 0, "and the decoder kept up");
}
