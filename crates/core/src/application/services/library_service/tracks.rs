//! The tracks in the library: listing them, taking them out and bringing them back, their genres and tags.

use super::*;

impl LibraryService {
    /// Everything currently in the active profile's library.
    pub fn tracks(&self) -> Result<Vec<Track>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.tracks.list_for_profile(profile_id)
    }

    /// Removes a track from the active profile's library.
    ///
    /// The file stays on disk and in the catalogue, and other profiles keep
    /// their copy.
    pub fn remove_track(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .tracks
            .remove(profile_id, media_file_id, self.context.now())?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// What this listener has taken out of their library.
    ///
    /// A removal is a decision, and a decision nobody can see is a decision
    /// nobody can undo. Until this existed the only way back was to add the
    /// folder again, which is a strange thing to have to work out.
    pub fn taken_out(&self) -> Result<Vec<TrackSummary>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.tracks.removed_for_profile(profile_id)
    }

    /// Puts one back.
    pub fn restore_track(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.tracks.restore(profile_id, media_file_id)?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Takes every removal whose file has gone off the list for good.
    ///
    /// Returns how many it forgot.
    ///
    /// **Only the ones with no file left.** A tombstone is both an offer to
    /// undo and the record that keeps a removed track out of the next scan.
    /// With the file gone it is neither, just a row nobody can act on. With the
    /// file still there it is working, and forgetting it would put the track
    /// back at the next scan.
    ///
    /// **Never automatic**: a folder on an unplugged drive looks exactly like a
    /// deleted one, and the difference shows up when the drive comes back.
    pub fn forget_gone(&self) -> Result<usize> {
        let profile_id = self.context.require_active_profile()?;

        let mut forgotten = 0;
        for summary in self.ports.tracks.removed_for_profile(profile_id)? {
            if self.still_there(summary.media_file_id) {
                continue;
            }

            self.ports
                .tracks
                .forget(profile_id, summary.media_file_id)?;
            forgotten += 1;
        }

        if forgotten > 0 {
            self.context.events.publish(DomainEvent::LibraryChanged);
        }

        Ok(forgotten)
    }

    /// How many of this listener's removals have no file left behind them.
    pub fn gone_for_good(&self) -> Result<usize> {
        let profile_id = self.context.require_active_profile()?;

        Ok(self
            .ports
            .tracks
            .removed_for_profile(profile_id)?
            .into_iter()
            .filter(|summary| !self.still_there(summary.media_file_id))
            .count())
    }

    /// The active profile's library, ready to be listed.
    ///
    /// The same content as [`Self::tracks`] with artist and album names resolved.
    /// The interface wants names; editing wants identifiers.
    pub fn summaries(&self) -> Result<Vec<TrackSummary>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.tracks.summaries_for_profile(profile_id)
    }

    /// Genres of a track as the active profile sees them.
    pub fn genres_of(&self, media_file_id: MediaFileId) -> Result<Vec<Genre>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .genres
            .for_profile_track(profile_id, media_file_id)
    }

    /// Corrects the genres of a track for the active profile only.
    ///
    /// The file's own genres are left as its tags describe them, and no other
    /// profile is affected: a correction is one listener's opinion about a
    /// recording they share.
    ///
    /// An empty list is a decision, not a reset — it means this listener wants
    /// the track filed under nothing. [`Self::reset_genres`] is the reset.
    pub fn set_genres(&self, media_file_id: MediaFileId, names: &[String]) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let ids = self.genre_ids(names)?;

        self.ports
            .genres
            .set_for_profile_track(profile_id, media_file_id, &ids)?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Drops the active profile's correction, restoring the file's own genres.
    pub fn reset_genres(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        self.ports
            .genres
            .clear_for_profile_track(profile_id, media_file_id)?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Corrects what this profile calls a track.
    ///
    /// A local override and nothing else: the file keeps its tags, the
    /// catalogue keeps its reading of them, and another profile sharing the
    /// same file goes on seeing what it always saw. There is no writing back to
    /// disk and there is not meant to be.
    ///
    /// An empty artist or album means "no artist", not "an artist called
    /// nothing": the columns already carry that distinction and a listener
    /// clearing a field is using it.
    pub fn edit_track(
        &self,
        media_file_id: MediaFileId,
        title: &str,
        artist: Option<&str>,
        album: Option<&str>,
    ) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::invalid(
                "track title",
                "a track needs something to be called",
            ));
        }

        let track = self
            .ports
            .tracks
            .get(profile_id, media_file_id)?
            .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

        let now = self.context.now();
        let artist_id = self.resolve_artist(blank_as_absent(artist), now)?;

        // The album is looked up under the artist it is now filed with, which
        // is what keeps two albums of the same name by different people apart.
        let album_id = self.resolve_album(blank_as_absent(album), artist_id, track.year, now)?;

        self.ports.tracks.save(&Track {
            title: title.to_owned(),
            artist_id,
            album_id,
            ..track
        })?;

        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }
}
