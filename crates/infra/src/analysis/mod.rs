//! Local DSP analysis: what a recording sounds like, in numbers.
//!
//! Every feature here is arithmetic over a spectrum. **No model, nothing
//! downloaded, nothing sent** — the no-neural-networks rule as an architecture
//! rather than a promise: there is no code here that could load a model.
//!
//! Read a window ([`decode`]), transform it once ([`dsp`]), then read every
//! feature off the same frames — [`energy`], [`spectral`], [`bpm`], [`key`] —
//! with [`danceability`] and [`valence`] derived rather than measured again.
//! [`feature_extractor`] assembles a row; [`worker`] paces the whole thing so
//! nobody notices a library being analysed.

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
pub mod waveform;
pub mod worker;

pub use feature_extractor::{DspFeatureExtractor, VERSION};
pub use waveform::WaveformStore;
pub use worker::AnalysisWorker;
