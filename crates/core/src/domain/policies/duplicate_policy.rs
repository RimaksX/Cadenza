//! Deciding whether two files are the same recording.

use crate::domain::media_file::MediaFile;

/// The relationship between a newly seen file and one already catalogued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateVerdict {
    /// Different recordings.
    Distinct,
    /// The same path — a rescan of a file already known, not a duplicate.
    SameFile,
    /// Byte-identical content at two different paths.
    SameContent,
}

impl DuplicateVerdict {
    /// True when the listener must be asked what to do.
    pub const fn needs_review(self) -> bool {
        matches!(self, Self::SameContent)
    }
}

/// Compares a candidate against a catalogued file.
///
/// Content hashes only. Fuzzy matching on title, artist and duration — which
/// would also catch the same song at two bitrates — is deliberately not done
/// here: it produces false positives, and every false positive costs the
/// listener a review-queue decision. If exact hashing turns out to miss too
/// much, add a second, clearly separate verdict for probable matches rather than
/// loosening this one.
///
/// A file with no hash yet compares as [`DuplicateVerdict::Distinct`]; the check
/// runs again once hashing completes.
pub fn compare(candidate: &MediaFile, existing: &MediaFile) -> DuplicateVerdict {
    if candidate.path == existing.path {
        return DuplicateVerdict::SameFile;
    }
    match (&candidate.file_hash, &existing.file_hash) {
        (Some(left), Some(right)) if left == right => DuplicateVerdict::SameContent,
        _ => DuplicateVerdict::Distinct,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DuplicateVerdict, compare};
    use crate::domain::ids::MediaFileId;
    use crate::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
    use crate::domain::value_objects::{DurationMs, Timestamp};

    fn file(path: &str, hash: Option<&str>) -> MediaFile {
        MediaFile {
            id: MediaFileId::new(),
            path: PathBuf::from(path),
            file_hash: hash.map(str::to_owned),
            file_size: 41_231_884,
            file_mtime: Timestamp::from_millis(1_754_611_200_000),
            format: AudioFormat::Flac,
            properties: AudioProperties {
                duration: DurationMs::from_secs(215),
                sample_rate: 44_100,
                channels: 2,
                bitrate: None,
            },
            metadata_version: None,
            metadata_extracted_at: None,
            state: FileState::Available,
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_same_path_is_a_rescan_not_a_duplicate() {
        let verdict = compare(
            &file("D:/Music/a.flac", Some("abc")),
            &file("D:/Music/a.flac", Some("abc")),
        );
        assert_eq!(verdict, DuplicateVerdict::SameFile);
        assert!(!verdict.needs_review());
    }

    #[test]
    fn identical_content_at_two_paths_needs_review() {
        let verdict = compare(
            &file("D:/Music/copy/a.flac", Some("abc")),
            &file("D:/Music/a.flac", Some("abc")),
        );
        assert_eq!(verdict, DuplicateVerdict::SameContent);
        assert!(verdict.needs_review());
    }

    #[test]
    fn different_content_is_distinct() {
        assert_eq!(
            compare(
                &file("D:/Music/b.flac", Some("def")),
                &file("D:/Music/a.flac", Some("abc")),
            ),
            DuplicateVerdict::Distinct
        );
    }

    #[test]
    fn an_unhashed_file_is_not_accused_of_being_a_duplicate() {
        assert_eq!(
            compare(
                &file("D:/Music/b.flac", None),
                &file("D:/Music/a.flac", Some("abc")),
            ),
            DuplicateVerdict::Distinct
        );
    }
}
