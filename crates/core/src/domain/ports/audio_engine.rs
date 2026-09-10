//! Controlling playback.

use std::path::Path;

use crate::Result;
use crate::domain::eq::EqSetting;
use crate::domain::playback::{PlaybackState, TransitionProfile};
use crate::domain::settings::CrossfadeDuration;
use crate::domain::value_objects::{PlaybackPosition, Volume};

/// The audio output and its graph.
///
/// Every method here is a *control-plane* call made from an application thread.
/// None of them runs on the realtime audio callback: implementations publish
/// parameters through atomics or lock-free queues and return immediately. The
/// callback itself must never allocate, block, perform IO or call back into
/// this trait.
pub trait AudioEnginePort: Send + Sync {
    /// Loads a file as the current track, replacing whatever was loaded.
    fn load(&self, path: &Path) -> Result<()>;

    /// Hands the next track to the engine so it can be decoded ahead of time.
    ///
    /// This is what makes both gapless joins and crossfades possible: the
    /// outgoing and incoming streams have to coexist before the transition, not
    /// at it.
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

    /// Whether a following track is already open and waiting.
    ///
    /// The caller arms one when this is false. Asking rather than remembering:
    /// the engine drops what it had armed whenever the ground moves under it —
    /// a seek, a new track loaded — and only the engine knows when that was.
    fn armed(&self) -> bool;

    /// How many times the engine has handed over to a preloaded track by itself.
    ///
    /// A count rather than a flag, so a caller that missed a tick still sees
    /// that it missed one. It is the only way the queue learns that the track
    /// it believes is playing has already given way to the next: with a join
    /// there is no moment of silence for anyone to notice.
    fn advances(&self) -> u64;

    /// Sets the crossfade length used for [`TransitionProfile::Crossfade`].
    fn set_crossfade(&self, duration: CrossfadeDuration) -> Result<()>;

    /// Applies an equaliser setting.
    ///
    /// The whole setting rather than one control, because a band is three
    /// numbers that only mean anything together. The engine walks its filters
    /// towards these rather than snapping to them, which is what keeps a moving
    /// control silent.
    fn set_eq(&self, setting: &EqSetting) -> Result<()>;

    /// The current position, as last reported by the engine.
    fn position(&self) -> PlaybackPosition;

    /// Whether audio is running.
    fn state(&self) -> PlaybackState;
}
