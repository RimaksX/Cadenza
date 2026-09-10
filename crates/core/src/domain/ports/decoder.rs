//! Identifying what a file actually contains.

use std::path::Path;

use crate::Result;
use crate::domain::media_file::{AudioFormat, AudioProperties};

/// What probing a file revealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeResult {
    /// The codec found inside the container.
    pub format: AudioFormat,
    /// Duration, sample rate, channels and bitrate.
    pub properties: AudioProperties,
}

/// Inspects audio files.
///
/// Probing is separated from playback on purpose. The scanner needs a file's
/// duration and sample rate for thousands of files without ever producing a
/// sample, and `.m4a` cannot be resolved to AAC or ALAC without opening it.
///
/// The streaming half of decoding — pulling PCM frames for the audio graph —
/// belongs to the engine. Keeping the port split this way is also what makes an
/// alternative decoder adapter possible later without touching the library
/// code.
pub trait DecoderPort: Send + Sync {
    /// Reads the stream header.
    fn probe(&self, path: &Path) -> Result<ProbeResult>;

    /// True when this decoder can handle the format.
    fn supports(&self, format: AudioFormat) -> bool;
}
