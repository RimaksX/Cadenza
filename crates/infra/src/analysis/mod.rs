//! Local DSP analysis: what a recording sounds like, in numbers.
//!
//! Every feature here is arithmetic over a spectrum — a correlation, a period,
//! an average. There is no model, nothing is downloaded and nothing is sent
//! anywhere, which is the no-neural-networks rule stated as an architecture
//! rather than as a promise: there is no code here that could load a model even
//! if somebody wanted one.
//!
//! The work is arranged as: read a window of the file ([`decode`]), transform it
//! once ([`dsp`]), and read every feature off the same frames — level
//! ([`energy`]), brightness ([`spectral`]), tempo ([`bpm`]), key ([`key`]) —
//! with [`danceability`] and [`valence`] derived from those rather than measured
//! again. [`feature_extractor`] assembles one row, and [`worker`] paces the
//! whole thing so that a library being analysed is a library nobody notices
//! being analysed.

pub mod activity;
pub mod bpm;
pub mod danceability;
pub mod decode;
pub mod dsp;
pub mod energy;
pub mod feature_extractor;
pub mod key;
pub mod spectral;
pub mod valence;
pub mod worker;

pub use feature_extractor::{DspFeatureExtractor, VERSION};
pub use worker::AnalysisWorker;
