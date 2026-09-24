//! Album release.

use super::ids::{AlbumId, ArtistId};
use super::value_objects::Timestamp;

/// An album in the global catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    /// Stable identifier.
    pub id: AlbumId,
    /// Album artist, absent for compilations with no single credited artist.
    pub artist_id: Option<ArtistId>,
    /// Album title.
    pub title: String,
    /// Release year, when the tags carry one.
    pub year: Option<u16>,
    /// When the row was created.
    pub created_at: Timestamp,
    /// When the row last changed.
    pub updated_at: Timestamp,
}
