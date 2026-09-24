//! The library by artist.

use std::collections::BTreeSet;

use super::*;
use crate::domain::policies::collection_policy;

/// An artist in the library, with their tracks in the order they play.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtistSummary {
    pub id: ArtistId,
    pub name: String,
    /// How many albums their tracks are on.
    pub albums: usize,
    pub tracks: Vec<TrackSummary>,
}

impl LibraryService {
    /// Every artist the library holds a track by, by name.
    pub fn artists(&self) -> Result<Vec<ArtistSummary>> {
        let (tracks, library) = self.tracks_and_rows()?;
        let ids: BTreeSet<String> = tracks
            .iter()
            .filter(|track| track.is_in_library())
            .filter_map(|track| track.artist_id.map(|id| id.to_string()))
            .collect();
        let mut artists = Vec::new();
        for id in ids {
            if let Some(artist) = self.artist_from(ArtistId::parse(&id)?, &tracks, &library)? {
                artists.push(artist);
            }
        }
        artists.sort_by_key(|artist| artist.name.to_lowercase());
        Ok(artists)
    }

    /// One artist, as [`Self::artists`] lists them.
    pub fn artist(&self, id: ArtistId) -> Result<ArtistSummary> {
        let (tracks, library) = self.tracks_and_rows()?;
        self.artist_from(id, &tracks, &library)?
            .ok_or_else(|| CoreError::not_found("artist", id))
    }

    fn tracks_and_rows(&self) -> Result<(Vec<Track>, Vec<TrackSummary>)> {
        let profile_id = self.context.require_active_profile()?;
        Ok((
            self.ports.tracks.list_for_profile(profile_id)?,
            self.ports.tracks.summaries_for_profile(profile_id)?,
        ))
    }

    fn artist_from(
        &self,
        id: ArtistId,
        tracks: &[Track],
        library: &[TrackSummary],
    ) -> Result<Option<ArtistSummary>> {
        let Some(artist) = self.ports.artists.get(id)? else {
            return Ok(None);
        };
        let rows = in_order(collection_policy::artist_order(tracks, id), library);
        if rows.is_empty() {
            return Ok(None);
        }
        let albums = tracks
            .iter()
            .filter(|track| track.is_in_library() && track.artist_id == Some(id))
            .filter_map(|track| track.album_id.map(|album| album.to_string()))
            .collect::<BTreeSet<_>>()
            .len();
        Ok(Some(ArtistSummary {
            id,
            name: artist.name,
            albums,
            tracks: rows,
        }))
    }
}

/// The rows of `library` for `order`, in that order.
fn in_order(order: Vec<MediaFileId>, library: &[TrackSummary]) -> Vec<TrackSummary> {
    order
        .into_iter()
        .filter_map(|id| {
            library
                .iter()
                .find(|summary| summary.media_file_id == id)
                .cloned()
        })
        .collect()
}
