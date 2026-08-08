//! A physical audio file on disk.

use std::fmt;
use std::path::PathBuf;

use super::ids::MediaFileId;
use super::value_objects::{DurationMs, Timestamp};
use crate::{CoreError, Result};

/// File extensions the scanner will consider.
///
/// `.m4a` is a container, not a codec: it may hold AAC or ALAC. Which one it is
/// cannot be told from the name, so the scanner accepts the extension and the
/// decoder probe decides the [`AudioFormat`].
pub const SUPPORTED_EXTENSIONS: [&str; 6] = ["mp3", "m4a", "aac", "alac", "flac", "wav"];

/// True when a file extension is worth opening.
pub fn is_supported_extension(extension: &str) -> bool {
    let lowered = extension.to_ascii_lowercase();
    SUPPORTED_EXTENSIONS.contains(&lowered.as_str())
}

/// The codec a file is encoded with (PROJECT_MASTER 2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    /// MPEG-1/2 Audio Layer III.
    Mp3,
    /// Advanced Audio Coding.
    Aac,
    /// Apple Lossless.
    Alac,
    /// Free Lossless Audio Codec.
    Flac,
    /// Uncompressed PCM in a RIFF container.
    Wav,
}

impl AudioFormat {
    /// The text form stored in `media_files.format`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Aac => "aac",
            Self::Alac => "alac",
            Self::Flac => "flac",
            Self::Wav => "wav",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "mp3" => Ok(Self::Mp3),
            "aac" => Ok(Self::Aac),
            "alac" => Ok(Self::Alac),
            "flac" => Ok(Self::Flac),
            "wav" => Ok(Self::Wav),
            other => Err(CoreError::invalid(
                "format",
                format!("unsupported format {other:?}"),
            )),
        }
    }

    /// True for formats that survive a decode/encode round trip unchanged.
    pub const fn is_lossless(self) -> bool {
        matches!(self, Self::Alac | Self::Flac | Self::Wav)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the file is still where the library last saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileState {
    /// Present and readable.
    #[default]
    Available,
    /// The path no longer resolves.
    Missing,
    /// Present but unreadable or undecodable.
    Errored,
}

impl FileState {
    /// The text form stored in `media_files.file_state`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "ok",
            Self::Missing => "missing",
            Self::Errored => "error",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "ok" => Ok(Self::Available),
            "missing" => Ok(Self::Missing),
            "error" => Ok(Self::Errored),
            other => Err(CoreError::invalid(
                "file state",
                format!("unknown state {other:?}"),
            )),
        }
    }

    /// True when the file can be queued for playback.
    ///
    /// Missing and errored files are excluded from shuffle and radio pools
    /// (PROJECT_MASTER 9.2).
    pub const fn is_playable(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Technical properties read from the stream itself, not from tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioProperties {
    /// Total playing time.
    pub duration: DurationMs,
    /// Samples per second, e.g. 44_100.
    pub sample_rate: u32,
    /// Channel count; 1 for mono, 2 for stereo.
    pub channels: u16,
    /// Average bitrate in bits per second, when the format reports one.
    pub bitrate: Option<u32>,
}

/// A physical audio file, shared by every profile that has it in their library.
///
/// This is global state: the file, its technical properties and its analysis
/// belong to the machine, not to a listener (PROJECT_MASTER 2.5). Per-profile
/// facts live in [`super::track::Track`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    /// Stable identifier.
    pub id: MediaFileId,
    /// Absolute path on disk. Unique across the catalogue.
    pub path: PathBuf,
    /// Content hash, used for duplicate detection. Absent until hashing runs.
    pub file_hash: Option<String>,
    /// Size in bytes.
    pub file_size: u64,
    /// Filesystem modification time, used to detect edits without rehashing.
    pub file_mtime: Timestamp,
    /// Codec, as determined by probing the stream.
    pub format: AudioFormat,
    /// Properties read from the stream.
    pub properties: AudioProperties,
    /// Version of the metadata reader that last ran, for re-read decisions.
    pub metadata_version: Option<String>,
    /// When tags were last read.
    pub metadata_extracted_at: Option<Timestamp>,
    /// Whether the file is still present and readable.
    pub state: FileState,
    /// When the row was created.
    pub created_at: Timestamp,
    /// When the row last changed.
    pub updated_at: Timestamp,
}

impl MediaFile {
    /// True when the file on disk differs from what was recorded.
    ///
    /// Size and mtime together are a cheap staleness check; only when this
    /// returns true does the scanner pay for rehashing or re-reading tags.
    pub fn is_stale(&self, current_size: u64, current_mtime: Timestamp) -> bool {
        self.file_size != current_size || self.file_mtime != current_mtime
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioFormat, FileState, is_supported_extension};

    #[test]
    fn extension_matching_ignores_case() {
        assert!(is_supported_extension("FLAC"));
        assert!(is_supported_extension("mp3"));
        assert!(!is_supported_extension("ogg"));
        assert!(!is_supported_extension("txt"));
    }

    #[test]
    fn format_text_form_round_trips() {
        for format in [
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::Alac,
            AudioFormat::Flac,
            AudioFormat::Wav,
        ] {
            assert_eq!(
                AudioFormat::parse(format.as_str()).expect("round trip"),
                format
            );
        }
        assert!(AudioFormat::parse("ogg").is_err());
    }

    #[test]
    fn only_available_files_are_playable() {
        assert!(FileState::Available.is_playable());
        assert!(!FileState::Missing.is_playable());
        assert!(!FileState::Errored.is_playable());
    }

    #[test]
    fn state_text_form_matches_the_schema_vocabulary() {
        assert_eq!(FileState::Available.as_str(), "ok");
        assert_eq!(FileState::parse("ok").expect("known"), FileState::Available);
        assert!(FileState::parse("gone").is_err());
    }
}
