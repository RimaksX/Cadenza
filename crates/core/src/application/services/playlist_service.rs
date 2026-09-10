//! Lists the listener keeps.
//!
//! A playlist is membership and order, and nothing else: no copies of files, no
//! per-playlist metadata about a track. Removing a track from a playlist leaves
//! it in the library, and deleting the playlist leaves everything in it alone.

use std::path::PathBuf;
use std::sync::Arc;

use super::cover;
use crate::application::context::AppContext;
use crate::domain::ids::{MediaFileId, PlaylistId, PlaylistItemId, ProfileId};
use crate::domain::playlist::{
    FAVOURITES_MIN_PLAYS, FAVOURITES_NAME, FAVOURITES_RULE, FAVOURITES_SIZE, Playlist, PlaylistItem,
};
use crate::domain::policies::retention_policy;
use crate::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::file_system::FileSystemPort;
use crate::domain::ports::folder_picker::FolderPickerPort;
use crate::domain::ports::repositories::{
    PlaylistRepositoryPort, StatsRepositoryPort, TrackRepositoryPort,
};
use crate::domain::profile::HISTORY_RETENTION_DAYS;
use crate::domain::track::TrackSummary;
use crate::domain::value_objects::DurationMs;
use crate::{CoreError, Result};

/// Everything playlists talk to.
pub struct PlaylistPorts {
    /// Where playlists are kept.
    pub playlists: Arc<dyn PlaylistRepositoryPort>,
    /// The library, for the titles a playlist's entries stand for.
    pub tracks: Arc<dyn TrackRepositoryPort>,
    /// Where a list's cover is kept, when somebody has given it one.
    pub artwork: Arc<dyn ArtworkCachePort>,
    /// The chooser that cover comes from.
    pub picker: Arc<dyn FolderPickerPort>,
    /// For reading the picture that was chosen, and nothing else.
    pub files: Arc<dyn FileSystemPort>,
    /// What has been played, for the favourites list to be built from.
    pub stats: Arc<dyn StatsRepositoryPort>,
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
    /// How many of its entries the profile can still play.
    ///
    /// Entries whose file has left the library are not counted, so that this
    /// agrees with the list the playlist's own page shows rather than with a
    /// row count nobody can act on.
    pub track_count: usize,
    /// How long those entries run.
    pub duration: DurationMs,
    /// The cover somebody gave it, if anybody did.
    ///
    /// A playlist has no file to take one from, so this is empty until it is
    /// chosen — unlike a track, which usually arrives carrying one.
    pub cover: Option<PathBuf>,
}

impl PlaylistService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: PlaylistPorts) -> Self {
        Self { context, ports }
    }

    /// Every playlist the active profile owns, with its size and its length.
    ///
    /// The library is read once for all of them: resolving each entry's
    /// duration playlist by playlist would turn a page of six cards into six
    /// listing queries.
    pub fn list(&self) -> Result<Vec<PlaylistSummary>> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        self.ports
            .playlists
            .list_for_profile(profile_id)?
            .into_iter()
            .map(|playlist| {
                let playable = self
                    .ports
                    .playlists
                    .items(playlist.id)?
                    .into_iter()
                    .filter_map(|item| {
                        library
                            .iter()
                            .find(|summary| summary.media_file_id == item.media_file_id)
                    });

                let mut track_count = 0;
                let mut duration = DurationMs::ZERO;
                for summary in playable {
                    track_count += 1;
                    duration = duration.saturating_add(summary.duration);
                }

                Ok(PlaylistSummary {
                    cover: self.ports.artwork.path_for(CoverOf::Playlist(playlist.id)),
                    playlist,
                    track_count,
                    duration,
                })
            })
            .collect::<Result<Vec<PlaylistSummary>>>()
            .map(|mut summaries| {
                // The favourites list first, because it is the one a listener
                // did not have to make and the one they will reach for most.
                // A stable sort, so everything else keeps the order the
                // repository gave it.
                summaries.sort_by_key(|summary| !summary.playlist.is_favourites());
                summaries
            })
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
        Self::not_the_favourites(&playlist, "renamed")?;
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
        Self::not_the_favourites(&playlist, "deleted")?;
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
        // A track already in the favourites list because it is played a lot
        // becomes a pinned one: the listener has now said so themselves, and
        // that outranks a count that could fall.
        if let Some(existing) = items
            .iter_mut()
            .find(|item| item.media_file_id == media_file_id && !item.by_hand)
        {
            existing.by_hand = true;
            return self.write_items(&playlist, &mut items);
        }

        items.push(PlaylistItem {
            id: PlaylistItemId::new(),
            playlist_id: playlist.id,
            media_file_id,
            position: u32::try_from(items.len()).unwrap_or(u32::MAX),
            added_at: self.context.clock.now(),
            by_hand: true,
        });

        self.write_items(&playlist, &mut items)
    }

    /// Removes the entry at a position, closing the gap it leaves.
    pub fn remove_at(&self, id: PlaylistId, position: usize) -> Result<()> {
        let playlist = self.owned(id)?;
        let mut items = self.ports.playlists.items(playlist.id)?;

        let Some(item) = items.get(position) else {
            return Err(CoreError::not_found("playlist entry", position));
        };

        // A row that is in the favourites list because it is played a lot
        // cannot be taken out of it: it would come back the next time the
        // counts moved, which is a worse answer than not offering.
        if playlist.is_favourites() && !item.by_hand {
            return Err(CoreError::invalid(
                "favourites",
                "this one is here because you keep playing it",
            ));
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

    /// The profile's favourites list, made if it is not there yet.
    ///
    /// Made on demand rather than by a migration, because it belongs to a
    /// profile and a migration cannot know how many of those there will be.
    pub fn favourites(&self) -> Result<Playlist> {
        let profile_id = self.context.require_active_profile()?;

        if let Some(existing) = self
            .ports
            .playlists
            .list_for_profile(profile_id)?
            .into_iter()
            .find(Playlist::is_favourites)
        {
            return Ok(existing);
        }

        let now = self.context.clock.now();
        let playlist = Playlist {
            id: PlaylistId::new(),
            profile_id,
            name: self.free_name(profile_id, FAVOURITES_NAME)?,
            description: None,
            is_smart: true,
            rule_json: Some(FAVOURITES_RULE.to_owned()),
            created_at: now,
            updated_at: now,
        };

        self.ports.playlists.save(&playlist)?;
        self.announce();
        Ok(playlist)
    }

    /// True when a track is in the profile's favourites, however it got there.
    ///
    /// However it got there, because that is what the heart on the player bar
    /// is asked to say: the listener wants to know whether this song is in
    /// their favourites, and "yes, but by arithmetic" is not a different
    /// answer to that question.
    pub fn is_favourite(&self, media_file_id: MediaFileId) -> Result<bool> {
        let playlist = self.favourites()?;
        Ok(self
            .ports
            .playlists
            .items(playlist.id)?
            .iter()
            .any(|item| item.media_file_id == media_file_id))
    }

    /// Puts a track in the favourites, or takes it out again.
    ///
    /// Taking out means taking the pin off. A track the count put there cannot
    /// be taken out at all - it would come back the next time a track ended -
    /// and this refuses in the same words the playlist page does rather than
    /// appearing to work. A track that was both pinned and earned stays in the
    /// list when its pin comes off, because it is still something the listener
    /// keeps playing.
    pub fn set_favourite(&self, media_file_id: MediaFileId, wanted: bool) -> Result<()> {
        let playlist = self.favourites()?;

        if wanted {
            return self.add_track(playlist.id, media_file_id);
        }

        let mut items = self.ports.playlists.items(playlist.id)?;
        let pinned = items
            .iter()
            .any(|item| item.media_file_id == media_file_id && item.by_hand);

        if !pinned {
            return if items.iter().any(|item| item.media_file_id == media_file_id) {
                Err(CoreError::invalid(
                    "favourites",
                    "this one is here because you keep playing it",
                ))
            } else {
                // Not in the list at all: there is nothing to undo, and saying
                // so would be an error message about a state the listener is
                // already in.
                Ok(())
            };
        }

        items.retain(|item| !(item.media_file_id == media_file_id && item.by_hand));
        self.write_items(&playlist, &mut items)?;
        // What the pin was holding up may still be earned by the count, and
        // the rebuild is what works that out.
        self.refresh_favourites()
    }

    /// Rebuilds the played part of the favourites list.
    ///
    /// Cheap enough to call whenever a track finishes: one grouped count over a
    /// month of listens and one rewrite of at most fifty rows. It writes
    /// nothing when the answer has not changed, so the ordinary case costs two
    /// reads and raises no event.
    ///
    /// What was pinned stays pinned and stays on top, in the order it was
    /// pinned. What follows is what the listener has actually come back to,
    /// most-played first. Only finished listens count - `top_tracks` is where
    /// that is decided - so a track skipped forty times is not a favourite.
    pub fn refresh_favourites(&self) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let playlist = self.favourites()?;

        let existing = self.ports.playlists.items(playlist.id)?;
        let pinned: Vec<PlaylistItem> = existing
            .iter()
            .filter(|item| item.by_hand)
            .cloned()
            .collect();

        // The whole retained window, which is the whole of what is known: the
        // history is thirty days by policy and this counts what is inside it. A
        // count that outlived the history would be a record of listening kept
        // after the listening itself was forgotten, and that is not a decision
        // to take quietly.
        let cutoff = retention_policy::cutoff(self.context.clock.now(), HISTORY_RETENTION_DAYS);

        let library = self.ports.tracks.summaries_for_profile(profile_id)?;
        let now = self.context.clock.now();
        let played: Vec<PlaylistItem> = self
            .ports
            .stats
            .top_tracks(profile_id, cutoff, FAVOURITES_SIZE)?
            .into_iter()
            .filter(|(_, plays)| *plays >= FAVOURITES_MIN_PLAYS)
            // A track that has left the library is not a favourite any more:
            // there is nothing left to play.
            .filter(|(media_file_id, _)| {
                library
                    .iter()
                    .any(|summary| summary.media_file_id == *media_file_id)
            })
            .filter(|(media_file_id, _)| {
                !pinned
                    .iter()
                    .any(|item| item.media_file_id == *media_file_id)
            })
            .map(|(media_file_id, _)| PlaylistItem {
                id: PlaylistItemId::new(),
                playlist_id: playlist.id,
                media_file_id,
                position: 0,
                added_at: now,
                by_hand: false,
            })
            .collect();

        let mut wanted = pinned;
        wanted.extend(played);

        // Identifiers are fresh every time this runs, so comparing the rows
        // themselves would always find a difference. What matters is which
        // tracks are there and in what order.
        let before: Vec<MediaFileId> = existing.iter().map(|item| item.media_file_id).collect();
        let after: Vec<MediaFileId> = wanted.iter().map(|item| item.media_file_id).collect();
        if before == after {
            return Ok(());
        }

        self.write_items(&playlist, &mut wanted)
    }

    /// Refuses an edit that would turn the favourites list into something else.
    fn not_the_favourites(playlist: &Playlist, what: &str) -> Result<()> {
        if playlist.is_favourites() {
            return Err(CoreError::invalid(
                "favourites",
                format!("the favourites list cannot be {what}"),
            ));
        }
        Ok(())
    }

    /// `wanted`, or the first free name after it.
    ///
    /// Names are unique per profile in the schema, and a listener may already
    /// have a list called Favourites - made by hand, before this existed. Theirs
    /// is theirs; the automatic one takes the next name along.
    fn free_name(&self, profile_id: ProfileId, wanted: &str) -> Result<String> {
        if self.find_by_name(profile_id, wanted)?.is_none() {
            return Ok(wanted.to_owned());
        }
        for suffix in 2..100 {
            let candidate = format!("{wanted} {suffix}");
            if self.find_by_name(profile_id, &candidate)?.is_none() {
                return Ok(candidate);
            }
        }
        Err(CoreError::invalid(
            "playlist name",
            "every name this list could take is taken",
        ))
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
    /// Playlists are never shared, so an identifier from
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

    fn find_by_name(&self, profile_id: ProfileId, name: &str) -> Result<Option<Playlist>> {
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
    /// Asks the listener for a picture and makes it this list's cover.
    ///
    /// `Ok(false)` means the chooser was closed, which is an answer.
    pub fn choose_cover(&self, playlist_id: PlaylistId) -> Result<bool> {
        // Read first: a cover may only be put on a list that belongs to whoever
        // is listening, and this is what says so.
        self.owned(playlist_id)?;

        let chosen = cover::choose(
            &cover::CoverPorts {
                artwork: Arc::clone(&self.ports.artwork),
                picker: Arc::clone(&self.ports.picker),
                files: Arc::clone(&self.ports.files),
            },
            CoverOf::Playlist(playlist_id),
            "Choose a cover",
        )?;
        if chosen {
            self.context.events.publish(DomainEvent::PlaylistsChanged);
        }
        Ok(chosen)
    }

    /// Takes the cover off a list, leaving the square it was in.
    pub fn clear_cover(&self, playlist_id: PlaylistId) -> Result<()> {
        self.owned(playlist_id)?;

        self.ports.artwork.remove(CoverOf::Playlist(playlist_id))?;
        self.context.events.publish(DomainEvent::PlaylistsChanged);
        Ok(())
    }
}
