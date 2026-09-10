//! A track as one profile sees it, plus the audio features derived from its file.

use super::ids::{AlbumId, ArtistId, MediaFileId, ProfileId};
use super::value_objects::{Bpm, DurationMs, MusicalKey, Timestamp};

/// A media file as it appears in one profile's library.
///
/// Titles, artist and album links here are the *effective* values: the scanner
/// seeds them from tags and the user may edit them, which overrides the tags
/// without touching the file. Two profiles can therefore disagree about the
/// same file, which is the point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Owning profile.
    pub profile_id: ProfileId,
    /// The physical file.
    pub media_file_id: MediaFileId,
    /// Effective title.
    pub title: String,
    /// Effective track artist.
    pub artist_id: Option<ArtistId>,
    /// Effective album.
    pub album_id: Option<AlbumId>,
    /// Position within the album.
    pub track_no: Option<u16>,
    /// Disc number for multi-disc releases.
    pub disc_no: Option<u16>,
    /// Release year.
    pub year: Option<u16>,
    /// When the profile added this track.
    pub added_at: Timestamp,
    /// When the profile removed it, if it has.
    ///
    /// Removal is a tombstone, not a delete: the file stays on disk and in the
    /// global catalogue, and other profiles keep their copy.
    pub removed_at: Option<Timestamp>,
}

impl Track {
    /// True when the track is currently part of the profile's library.
    pub const fn is_in_library(&self) -> bool {
        self.removed_at.is_none()
    }
}

/// A track with its artist and album resolved, ready to be listed.
///
/// A read model rather than an entity: nothing is saved through it. It exists
/// because [`Track`] holds identifiers where a listing needs names, and looking
/// each one up per row turns a two-thousand-track library into four thousand
/// queries. The repository resolves them in the same statement that reads the
/// rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSummary {
    /// The file this row plays.
    pub media_file_id: MediaFileId,
    /// Effective title, as this profile sees it.
    pub title: String,
    /// Artist name. Absent when the file carried no artist tag.
    pub artist: Option<String>,
    /// Album title. Absent when the file carried no album tag.
    pub album: Option<String>,
    /// Playing time, from the file's own stream properties.
    pub duration: DurationMs,
}

/// Audio features extracted by local DSP analysis.
///
/// Global, not per-profile: they describe the file. Every normalised field is
/// `0.0..=1.0`. Absent BPM or key means detection failed or has not run — callers
/// must treat that as "unknown" rather than substituting zero, which would read
/// as "no energy at all" and poison transition scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackFeatures {
    /// The file these features describe.
    pub media_file_id: MediaFileId,
    /// Detected tempo, absent when detection was not confident enough.
    pub bpm: Option<Bpm>,
    /// Confidence in the tempo estimate, `0.0..=1.0`.
    pub bpm_confidence: f32,
    /// Detected key and mode, absent when detection failed.
    pub key: Option<MusicalKey>,
    /// Perceived intensity, `0.0..=1.0`.
    pub energy: f32,
    /// Integrated loudness, normalised to `0.0..=1.0`.
    pub loudness: f32,
    /// Spectral centroid ("brightness"), normalised to `0.0..=1.0`.
    pub spectral_centroid: f32,
    /// Spectral rolloff, normalised to `0.0..=1.0`.
    pub spectral_rolloff: f32,
    /// Rhythmic regularity and beat strength, `0.0..=1.0`.
    pub danceability: f32,
    /// Estimated musical positivity, `0.0..=1.0`.
    pub valence: f32,
    /// How steady the tempo is across the track, `0.0..=1.0`.
    pub tempo_stability: f32,
    /// Difference between loud and quiet passages, normalised to `0.0..=1.0`.
    pub dynamic_range: f32,
    /// Version of the extractor that produced these values.
    ///
    /// Re-analysis happens only when this differs from the current extractor,
    /// which is what stops the background worker redoing settled work.
    pub extractor_version: String,
    /// When analysis ran.
    pub analyzed_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::Track;
    use crate::domain::ids::{MediaFileId, ProfileId};
    use crate::domain::value_objects::Timestamp;

    fn track(removed_at: Option<Timestamp>) -> Track {
        Track {
            profile_id: ProfileId::new(),
            media_file_id: MediaFileId::new(),
            title: "Untitled".to_owned(),
            artist_id: None,
            album_id: None,
            track_no: None,
            disc_no: None,
            year: None,
            added_at: Timestamp::from_millis(1_754_611_200_000),
            removed_at,
        }
    }

    #[test]
    fn removal_is_a_tombstone() {
        assert!(track(None).is_in_library());
        assert!(!track(Some(Timestamp::from_millis(1_754_697_600_000))).is_in_library());
    }
}
