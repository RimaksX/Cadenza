//! Deriving audio features with local DSP.

use std::path::Path;

use crate::Result;
use crate::domain::ids::MediaFileId;
use crate::domain::track::TrackFeatures;

/// Extracts BPM, key, energy and the spectral features radio depends on.
///
/// Local DSP only — no models, no downloads, no network.
/// Implementations run on a low-priority background thread and must
/// remain interruptible: analysing a five-thousand-track library must never make
/// playback stutter or the machine feel busy.
pub trait FeatureExtractorPort: Send + Sync {
    /// Identifier of this extractor's algorithm, stored in
    /// `track_features.extractor_version`.
    ///
    /// A file is re-analysed only when this differs from the stored value, which
    /// is what stops the worker redoing settled work on every startup.
    fn version(&self) -> &str;

    /// Analyses a file.
    fn extract(&self, media_file_id: MediaFileId, path: &Path) -> Result<TrackFeatures>;
}
