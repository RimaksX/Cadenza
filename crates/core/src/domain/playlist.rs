//! User-curated and rule-based playlists.

use super::ids::{MediaFileId, PlaylistId, PlaylistItemId, ProfileId};
use super::value_objects::Timestamp;
use crate::{CoreError, Result};

/// Longest playlist name accepted, in characters.
pub const MAX_PLAYLIST_NAME_CHARS: usize = 128;

/// What the automatic favourites list is called.
pub const FAVOURITES_NAME: &str = "Favourites";

/// The rule that marks a playlist as the automatic favourites list.
///
/// One exact string rather than a rule language. `rule_json` is still opaque to
/// the domain - nothing here parses it - and this is a comparison, not a
/// parser. When a second kind of automatic list exists there will be a reason
/// to invent the language; there is not one yet.
pub const FAVOURITES_RULE: &str = "{\"auto\":\"favourites\"}";

/// How many played tracks the favourites list holds at most.
///
/// Enough to be a record of a season's listening and few enough to still be a
/// recommendation. Pinned tracks are on top of this, not inside it.
pub const FAVOURITES_SIZE: u32 = 50;

/// How many finished listens a track needs before it counts as a favourite.
///
/// Two, not one. Everything gets played once; coming back to it is the thing
/// worth recording, and a threshold of one would make the list a list of the
/// library in the order it was imported.
pub const FAVOURITES_MIN_PLAYS: u32 = 2;

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
    /// playlists. Storing it as text here avoids inventing a rule AST now
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

    /// True when this is the profile's automatic favourites list.
    ///
    /// It cannot be renamed or deleted, for the reason the built-in moods
    /// cannot: it is the thing a listener gets back to, and a list that has
    /// been quietly turned into something else is not that.
    pub fn is_favourites(&self) -> bool {
        self.is_smart && self.rule_json.as_deref() == Some(FAVOURITES_RULE)
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
    /// True when a listener put this entry here.
    ///
    /// Always true for an ordinary playlist: somebody added every row of one.
    /// The favourites list is the only place the two can differ, and there the
    /// difference is what lets the played part be rebuilt without disturbing
    /// what was pinned.
    pub by_hand: bool,
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
