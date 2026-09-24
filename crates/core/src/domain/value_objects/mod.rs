//! Small validated types that make illegal states unrepresentable.
//!
//! Constructors reject out-of-range input rather than clamping silently, except
//! where a `clamped` helper is offered explicitly for values arriving from a UI
//! slider, where clamping is the friendlier behaviour.

pub mod bpm;
pub mod duration;
pub mod gain;
pub mod musical_key;
pub mod playback_position;
pub mod theme_mode;
pub mod timestamp;
pub mod volume;

pub use bpm::Bpm;
pub use duration::DurationMs;
pub use gain::GainDb;
pub use musical_key::{Mode, MusicalKey};
pub use playback_position::PlaybackPosition;
pub use theme_mode::ThemeMode;
pub use timestamp::Timestamp;
pub use volume::Volume;
