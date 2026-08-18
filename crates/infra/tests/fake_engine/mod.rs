//! An audio engine that plays nothing and remembers everything.
//!
//! Enough of one for tests about what gets written down rather than what gets
//! heard: it holds a position a test can move, and forgets it when a track is
//! loaded, which is what a real engine does.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cadenza_core::Result;
use cadenza_core::domain::eq::EqSetting;
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::settings::CrossfadeDuration;
use cadenza_core::domain::value_objects::{PlaybackPosition, Volume};

#[derive(Default)]
pub struct FakeEngine {
    loaded: Mutex<Option<PathBuf>>,
    playing: AtomicBool,
    position: Mutex<PlaybackPosition>,
}

impl AudioEnginePort for FakeEngine {
    fn load(&self, path: &Path) -> Result<()> {
        *self.loaded.lock().expect("not poisoned") = Some(path.to_path_buf());
        *self.position.lock().expect("not poisoned") = PlaybackPosition::START;
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
    fn set_eq(&self, _setting: &EqSetting) -> Result<()> {
        Ok(())
    }
    fn set_visualising(&self, _on: bool) {}
    fn spectrum(&self, _bars: &mut [f32]) -> bool {
        false
    }
    fn position(&self) -> PlaybackPosition {
        *self.position.lock().expect("not poisoned")
    }
    fn state(&self) -> PlaybackState {
        if self.loaded.lock().expect("not poisoned").is_none() {
            return PlaybackState::Stopped;
        }
        if self.playing.load(Ordering::Relaxed) {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }
}
