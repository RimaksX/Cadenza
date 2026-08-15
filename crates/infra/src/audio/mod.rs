//! Turning files into sound (PROJECT_MASTER 8).
//!
//! The graph section 8.1 describes is built one milestone at a time. What exists
//! now is the spine of it:
//!
//! ```text
//! TrackStream -> channel map -> Resampling -> mixer -> SampleRing -> gain -> device
//! ```
//!
//! Two `TrackStream`s run at once through a transition, which is what the mixer
//! is there for. The EQ chain arrives in M9 and the visualiser tap in M10. Each of them inserts into this chain rather than replacing it,
//! which is why the ring sits where it does: everything before it is free to
//! allocate and block, and everything after it is not (PROJECT_MASTER 8.2).

mod crossfade;
pub mod engine;
pub mod resampler;
pub mod ring_buffer;
mod stream;
pub mod symphonia_decoder;

pub use engine::CpalAudioEngine;
pub use symphonia_decoder::{StreamInfo, SymphoniaDecoder, TrackStream};
