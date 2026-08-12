//! Lists the listener keeps.
//!
//! A playlist is membership and order, and nothing else: no copies of files, no
//! per-playlist metadata about a track. Removing a track from a playlist leaves
//! it in the library, and deleting the playlist leaves everything in it alone
//! (PROJECT_MASTER 2.6).

use std::sync::Arc;

use crate::application::context::AppContext;
use crate::domain::ids::{MediaFileId, PlaylistId, PlaylistItemId};
use crate::domain::playlist::{Playlist, PlaylistItem};
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::repositories::{PlaylistRepositoryPort, TrackRepositoryPort};
use crate::domain::track::TrackSummary;
use crate::{CoreError, Result};

/// Everything playlists talk to.
pub struct PlaylistPorts {
    /// Where playlists are kept.
    pub playlists: Arc<dyn PlaylistRepositoryPort>,
    /// The library, for the titles a playlist's entries stand for.
    pub tracks: Arc<dyn TrackRepositoryPort>,
}

/// Creating, editing and reading playlists.
pub struct PlaylistService {
    context: Arc<AppContext>,
    ports: PlaylistPorts,
}

/// A playlist and how much is in it, which is what a listing shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistSummary {
    /// The playlist itself.
    pub playlist: Playlist,
    /// How many entries it has.
    pub track_count: usize,
}

impl PlaylistService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: PlaylistPorts) -> Self {
        Self { context, ports }
    }

    /// Every playlist the active profile owns, with its size.
    pub fn list(&self) -> Result<Vec<PlaylistSummary>> {
        let profile_id = self.context.require_active_profile()?;

        self.ports
            .playlists
            .list_for_profile(profile_id)?
            .into_iter()
            .map(|playlist| {
                let track_count = self.ports.playlists.items(playlist.id)?.len();
                Ok(PlaylistSummary {
                    playlist,
                    track_count,
                })
            })
            .collect()
    }

    /// Creates an empty playlist.
    pub fn create(&self, name: &str) -> Result<Playlist> {
        let profile_id = self.context.require_active_profile()?;
        let name = Playlist::validate_name(name)?;

        // Names are unique per profile in the schema, so the same check here is
        // the difference between a message about a name and a message about a
        // constraint.
        if self.find_by_name(profile_id, &name)?.is_some() {
            return Err(CoreError::invalid(
                "playlist name",
                format!("{name:?} already exists"),
            ));
        }

        let now = self.context.clock.now();
        let playlist = Playlist {
            id: PlaylistId::new(),
            profile_id,
            name,
            description: None,
            is_smart: false,
            rule_json: None,
            created_at: now,
            updated_at: now,
        };

        self.ports.playlists.save(&playlist)?;
        self.announce();
        Ok(playlist)
    }

    /// Changes a playlist's name.
    pub fn rename(&self, id: PlaylistId, name: &str) -> Result<Playlist> {
        let mut playlist = self.owned(id)?;
        let name = Playlist::validate_name(name)?;

        if let Some(existing) = self.find_by_name(playlist.profile_id, &name)?
            && existing.id != id
        {
            return Err(CoreError::invalid(
                "playlist name",
                format!("{name:?} already exists"),
            ));
        }

        playlist.name = name;
        playlist.updated_at = self.context.clock.now();
        self.ports.playlists.save(&playlist)?;
        self.announce();
        Ok(playlist)
    }

    /// Deletes a playlist. The tracks stay in the library.
    pub fn delete(&self, id: PlaylistId) -> Result<()> {
        let playlist = self.owned(id)?;
        self.ports.playlists.delete(playlist.id)?;
        self.announce();
        Ok(())
    }

    /// Appends a track to the end of a playlist.
    ///
    /// The same track may be added twice: an entry has its own identity because
    /// a listener who puts one track at both ends of a set meant to.
    pub fn add_track(&self, id: PlaylistId, media_file_id: MediaFileId) -> Result<()> {
        let playlist = self.owned(id)?;
        self.in_library(media_file_id)?;

        let mut items = self.ports.playlists.items(playlist.id)?;
        items.push(PlaylistItem {
            id: PlaylistItemId::new(),
            playlist_id: playlist.id,
            media_file_id,
            position: u32::try_from(items.len()).unwrap_or(u32::MAX),
            added_at: self.context.clock.now(),
        });

        self.write_items(&playlist, &mut items)
    }

    /// Removes the entry at a position, closing the gap it leaves.
    pub fn remove_at(&self, id: PlaylistId, position: usize) -> Result<()> {
        let playlist = self.owned(id)?;
        let mut items = self.ports.playlists.items(playlist.id)?;

        if position >= items.len() {
            return Err(CoreError::not_found("playlist entry", position));
        }
        items.remove(position);

        self.write_items(&playlist, &mut items)
    }

    /// Moves an entry to another position, shifting the rest along.
    pub fn move_entry(&self, id: PlaylistId, from: usize, to: usize) -> Result<()> {
        let playlist = self.owned(id)?;
        let mut items = self.ports.playlists.items(playlist.id)?;

        if from >= items.len() {
            return Err(CoreError::not_found("playlist entry", from));
        }
        // Dropping past the end means the end, which is what dragging a row to
        // the bottom of a list looks like.
        let to = to.min(items.len().saturating_sub(1));

        let moved = items.remove(from);
        items.insert(to, moved);

        self.write_items(&playlist, &mut items)
    }

    /// A playlist's tracks, in playlist order, ready to list.
    ///
    /// Entries whose file has left the profile's library are skipped: the row
    /// has no title to draw and nothing to play.
    pub fn tracks_of(&self, id: PlaylistId) -> Result<Vec<TrackSummary>> {
        let playlist = self.owned(id)?;
        let profile_id = playlist.profile_id;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        Ok(self
            .ports
            .playlists
            .items(playlist.id)?
            .into_iter()
            .filter_map(|item| {
                library
                    .iter()
                    .find(|summary| summary.media_file_id == item.media_file_id)
                    .cloned()
            })
            .collect())
    }

    /// One playlist, if the active profile owns it.
    pub fn get(&self, id: PlaylistId) -> Result<Playlist> {
        self.owned(id)
    }

    /// Renumbers and stores the entries.
    fn write_items(&self, playlist: &Playlist, items: &mut [PlaylistItem]) -> Result<()> {
        for (position, item) in items.iter_mut().enumerate() {
            item.position = u32::try_from(position).unwrap_or(u32::MAX);
        }

        self.ports.playlists.replace_items(playlist.id, items)?;

        let mut touched = playlist.clone();
        touched.updated_at = self.context.clock.now();
        self.ports.playlists.save(&touched)?;

        self.announce();
        Ok(())
    }

    /// Reads a playlist and refuses one belonging to another profile.
    ///
    /// Playlists are never shared (PROJECT_MASTER 2.5), so an identifier from
    /// somewhere else is not found rather than forbidden: as far as this profile
    /// is concerned it does not exist.
    fn owned(&self, id: PlaylistId) -> Result<Playlist> {
        let profile_id = self.context.require_active_profile()?;

        self.ports
            .playlists
            .get(id)?
            .filter(|playlist| playlist.profile_id == profile_id)
            .ok_or_else(|| CoreError::not_found("playlist", id))
    }

    fn find_by_name(
        &self,
        profile_id: crate::domain::ids::ProfileId,
        name: &str,
    ) -> Result<Option<Playlist>> {
        Ok(self
            .ports
            .playlists
            .list_for_profile(profile_id)?
            .into_iter()
            .find(|playlist| playlist.name.eq_ignore_ascii_case(name)))
    }

    /// Refuses a track the profile does not have.
    fn in_library(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .tracks
            .summary(profile_id, media_file_id)?
            .ok_or_else(|| CoreError::not_found("track", media_file_id))?;
        Ok(())
    }

    fn announce(&self) {
        self.context.events.publish(DomainEvent::PlaylistsChanged);
    }
}
