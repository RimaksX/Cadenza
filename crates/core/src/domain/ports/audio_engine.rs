//! Controlling playback.

use std::path::Path;

use crate::Result;
use crate::domain::playback::{PlaybackState, TransitionProfile};
use crate::domain::settings::CrossfadeDuration;
use crate::domain::value_objects::{PlaybackPosition, Volume};

/// The audio output and its graph.
///
/// Every method here is a *control-plane* call made from an application thread.
/// None of them runs on the realtime audio callback: implementations publish
/// parameters through atomics or lock-free queues and return immediately. The
/// callback itself must never allocate, block, perform IO or call back into this
/// trait (PROJECT_MASTER 8.2).
pub trait AudioEnginePort: Send + Sync {
    /// Loads a file as the current track, replacing whatever was loaded.
    fn load(&self, path: &Path) -> Result<()>;

    /// Hands the next track to the engine so it can be decoded ahead of time.
    ///
    /// This is what makes both gapless joins and crossfades possible: the
    /// outgoing and incoming streams have to coexist before the transition, not
    /// at it (PROJECT_MASTER 8.4).
    fn preload_next(&self, path: &Path, transition: TransitionProfile) -> Result<()>;

    /// Starts or resumes output.
    fn play(&self) -> Result<()>;

    /// Holds position without unloading.
    fn pause(&self) -> Result<()>;

    /// Stops and unloads.
    fn stop(&self) -> Result<()>;

    /// Jumps to a position in the current track.
    fn seek(&self, position: PlaybackPosition) -> Result<()>;

    /// Sets the output level.
    fn set_volume(&self, volume: Volume) -> Result<()>;

    /// Sets the crossfade length used for [`TransitionProfile::Crossfade`].
    fn set_crossfade(&self, duration: CrossfadeDuration) -> Result<()>;

    /// The current position, as last reported by the engine.
    fn position(&self) -> PlaybackPosition;

    /// Whether audio is running.
    fn state(&self) -> PlaybackState;
}
