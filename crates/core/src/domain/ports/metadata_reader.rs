//! Reading tags and embedded artwork.

use std::path::Path;

use crate::Result;

/// Tag values as read from a file, before any per-profile override is applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadata {
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

/// Reads tags from audio files.
pub trait MetadataReaderPort: Send + Sync {
    /// Identifier of this reader's behaviour, stored in
    /// `media_files.metadata_version`.
    ///
    /// Files are only re-read when this changes, so bumping it is how a reader
    /// improvement gets applied to an existing library.
    fn version(&self) -> &str;

    /// Reads what tags the file has.
    ///
    /// Missing tags are absent fields, not an error: an untagged file is still a
    /// perfectly playable file and belongs in the library.
    fn read(&self, path: &Path) -> Result<TrackMetadata>;
}
