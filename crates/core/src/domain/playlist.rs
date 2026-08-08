//! User-curated and rule-based playlists.

use super::ids::{MediaFileId, PlaylistId, PlaylistItemId, ProfileId};
use super::value_objects::Timestamp;
use crate::{CoreError, Result};

/// Longest playlist name accepted, in characters.
pub const MAX_PLAYLIST_NAME_CHARS: usize = 128;

/// A playlist belonging to one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    /// Stable identifier.
    pub id: PlaylistId,
    /// Owning profile. Playlists are never shared between profiles.
    pub profile_id: ProfileId,
    /// Display name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// True when membership is computed from [`Self::rule_json`] rather than
    /// from explicit [`PlaylistItem`] rows.
    pub is_smart: bool,
    /// Serialised smart-playlist rule.
    ///
    /// Opaque to the domain on purpose: the rule language is defined with smart
    /// playlists in M7. Storing it as text here avoids inventing a rule AST now
    /// and rewriting it then.
    pub rule_json: Option<String>,
    /// When the playlist was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

impl Playlist {
    /// Trims and validates a playlist name.
    pub fn validate_name(raw: &str) -> Result<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CoreError::invalid("playlist name", "must not be empty"));
        }
        let length = trimmed.chars().count();
        if length > MAX_PLAYLIST_NAME_CHARS {
            return Err(CoreError::invalid(
                "playlist name",
                format!(
                    "{length} characters exceeds the {MAX_PLAYLIST_NAME_CHARS} character limit"
                ),
            ));
        }
        Ok(trimmed.to_owned())
    }

    /// True when membership is fixed rather than computed.
    ///
    /// Manual reordering and drag-and-drop apply only to these.
    pub const fn is_manually_ordered(&self) -> bool {
        !self.is_smart
    }
}

/// One entry in a manually curated playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    /// Stable identifier.
    ///
    /// Entries have their own id rather than being keyed by file, because the
    /// same track may legitimately appear twice in one playlist.
    pub id: PlaylistItemId,
    /// Owning playlist.
    pub playlist_id: PlaylistId,
    /// The file to play.
    pub media_file_id: MediaFileId,
    /// Zero-based position within the playlist.
    pub position: u32,
    /// When the entry was added.
    pub added_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::{MAX_PLAYLIST_NAME_CHARS, Playlist};

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(
            Playlist::validate_name("  Late night  ").expect("valid"),
            "Late night"
        );
        assert!(Playlist::validate_name("   ").is_err());
        assert!(Playlist::validate_name(&"x".repeat(MAX_PLAYLIST_NAME_CHARS + 1)).is_err());
    }
}
