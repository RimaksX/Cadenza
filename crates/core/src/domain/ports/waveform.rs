//! Measuring and keeping the shape of a track's loudness.

use crate::Result;
use crate::domain::ids::MediaFileId;

/// Where a track's waveform comes from.
///
/// Measured once, in the background, and kept: the whole file has to be
/// decoded to know its shape, which is work for a thread with nothing better
/// to do and not for the moment somebody presses play.
pub trait WaveformPort: Send + Sync {
    /// The kept shape of a file, [`WAVEFORM_POINTS`] levels as
    /// [`levels`] keeps them, if it has been measured.
    ///
    /// [`WAVEFORM_POINTS`]: crate::domain::waveform::WAVEFORM_POINTS
    /// [`levels`]: crate::domain::waveform::levels
    fn get(&self, media_file_id: MediaFileId) -> Result<Option<Vec<u8>>>;

    /// Asks for a file to be measured. Returns at once; the shape turns up in
    /// [`Self::get`] when it is done.
    fn request(&self, media_file_id: MediaFileId);
}
