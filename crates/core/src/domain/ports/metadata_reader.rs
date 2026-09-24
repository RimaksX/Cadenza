//! Reading tags, stream properties and embedded artwork.

use std::path::Path;

use crate::Result;
use crate::domain::media_file::{AudioFormat, AudioProperties};

/// Tag values as read from a file, before any per-profile override is applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackTags {
    /// Track title.
    pub title: Option<String>,
    /// Track artist.
    pub artist: Option<String>,
    /// Album artist, which may differ from the track artist on compilations.
    pub album_artist: Option<String>,
    /// Album title.
    pub album: Option<String>,
    /// Genre labels; files may legitimately carry several.
    pub genres: Vec<String>,
    /// Position within the album.
    pub track_no: Option<u16>,
    /// Disc number.
    pub disc_no: Option<u16>,
    /// Release year.
    pub year: Option<u16>,
    /// Embedded cover image bytes, if present.
    pub artwork: Option<Vec<u8>>,
}

/// Everything one pass over a file yields.
///
/// Tags and stream properties come back together because they come from the
/// same read. Splitting them into two port calls would mean opening and parsing
/// every file twice during a scan, for no benefit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    /// The codec found inside the container.
    ///
    /// Read from the stream, not guessed from the extension: `.m4a` may hold
    /// AAC or ALAC and the name does not say which.
    pub format: AudioFormat,
    /// Duration, sample rate, channels and bitrate.
    pub properties: AudioProperties,
    /// What the tags said.
    pub tags: TrackTags,
}

/// Reads audio files.
pub trait MetadataReaderPort: Send + Sync {
    /// Identifier of this reader's behaviour, stored in
    /// `media_files.metadata_version`.
    ///
    /// Files are only re-read when this changes, so bumping it is how a reader
    /// improvement gets applied to an existing library.
    fn version(&self) -> &str;

    /// Reads one file.
    ///
    /// Missing tags are absent fields, not an error: an untagged file is still a
    /// perfectly playable file and belongs in the library. An unreadable *stream*
    /// is an error, because without a duration and a sample rate there is
    /// nothing to play.
    fn read(&self, path: &Path) -> Result<FileMetadata>;
}
