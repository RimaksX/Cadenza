//! Turning files into sound.
//!
//! The graph is built one stage at a time. What exists
//! now is the spine of it:
//!
//! ```text
//! TrackStream -> channel map -> Resampling -> mixer -> SampleRing -> EQ -> gain
//!     -> device
//! ```
//!
//! Two `TrackStream`s run at once through a transition, which is what the mixer
//! is there for. The equaliser is downstream of the ring rather than upstream:
//! it is the only side where a slider is heard at once. Each stage inserts into
//! this chain rather than replacing it, which is why the ring sits where it
//! does: everything before it is free to allocate and block, and everything
//! after it is not.
//!
//! A visualiser tap used to sit after the gain, on what actually left for the
//! device. It is gone with the bars it fed.

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
