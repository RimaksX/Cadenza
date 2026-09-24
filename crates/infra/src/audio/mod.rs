//! Turning files into sound.
//!
//! The graph is built one stage at a time. What exists now is the spine of it:
//!
//! ```text
//! TrackStream -> channel map -> Resampling -> mixer -> SampleRing -> EQ -> gain
//!     -> device
//! ```
//!
//! Two `TrackStream`s run at once through a transition, which is what the mixer
//! is for. The equaliser sits downstream of the ring rather than upstream
//! because that is the only side where a slider is heard at once.
//!
//! **The ring is the boundary**: everything before it may allocate and block,
//! everything after it may not.

mod biquad;
mod crossfade;
pub mod engine;
mod eq;
pub mod resampler;
pub mod ring_buffer;
mod stream;
pub mod symphonia_decoder;

pub use engine::CpalAudioEngine;
pub use symphonia_decoder::{StreamInfo, SymphoniaDecoder, TrackStream};
