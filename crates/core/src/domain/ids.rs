//! Typed identifiers.
//!
//! Every entity is keyed by a UUID so that a future sync or mobile client can mint
//! identifiers without coordinating with anything. Each entity
//! gets its own type, so passing a playlist id where a track id is expected does not
//! compile.

use std::fmt;

use uuid::Uuid;

use crate::CoreError;

macro_rules! define_ids {
    ($($name:ident => $label:literal),* $(,)?) => {
        $(
            #[doc = concat!("Identifier of a ", $label, ".")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub struct $name(Uuid);

            impl $name {
                /// Mints a fresh random identifier.
                pub fn new() -> Self {
                    Self(Uuid::new_v4())
                }

                /// Wraps an existing UUID.
                pub const fn from_uuid(value: Uuid) -> Self {
                    Self(value)
                }

                /// Returns the underlying UUID.
                pub const fn as_uuid(self) -> Uuid {
                    self.0
                }

                /// Parses the hyphenated text form stored in the database.
                pub fn parse(text: &str) -> crate::Result<Self> {
                    Uuid::parse_str(text)
                        .map(Self)
                        .map_err(|err| CoreError::invalid($label, err.to_string()))
                }
            }

            impl Default for $name {
                fn default() -> Self {
                    Self::new()
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(&self.0, f)
                }
            }
        )*
    };
}

define_ids! {
    ProfileId => "profile id",
    ProfileFolderId => "profile folder id",
    MediaFileId => "media file id",
    ArtistId => "artist id",
    AlbumId => "album id",
    GenreId => "genre id",
    PlaylistId => "playlist id",
    PlaylistItemId => "playlist item id",
    PlayEventId => "play event id",
    RadioSessionId => "radio session id",
    RadioSessionItemId => "radio session item id",
    MoodId => "mood id",
    EqPresetId => "eq preset id",
    AnalysisJobId => "analysis job id",
    ImportReviewId => "import review id",
}

#[cfg(test)]
mod tests {
    use super::{MediaFileId, ProfileId};

    #[test]
    fn fresh_ids_are_distinct() {
        assert_ne!(ProfileId::new(), ProfileId::new());
    }

    #[test]
    fn text_form_round_trips() {
        let id = MediaFileId::new();
        let parsed = MediaFileId::parse(&id.to_string()).expect("its own text form parses");
        assert_eq!(id, parsed);
    }

    #[test]
    fn malformed_text_is_rejected() {
        let err = MediaFileId::parse("not-a-uuid").expect_err("garbage must not parse");
        assert!(err.to_string().contains("media file id"));
    }
}
