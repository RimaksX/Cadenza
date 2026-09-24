//! The pictures on the library's tracks.

use super::*;

impl LibraryService {
    /// The cover to show for a track: this listener's choice, else the file's.
    pub fn cover_for(&self, media_file_id: MediaFileId) -> Result<Option<PathBuf>> {
        let profile_id = self.context.require_active_profile()?;
        Ok(CoverOf::shown_for(
            self.ports.artwork.as_ref(),
            profile_id,
            media_file_id,
        ))
    }

    /// The same cover in the size a list draws, made on the first ask.
    ///
    /// A row shows a cover at forty-odd pixels and the stored ones are
    /// routinely a thousand square, so the list asks for this one and the
    /// player bar — which draws one cover, large — asks for the other.
    pub fn thumbnail_for(&self, media_file_id: MediaFileId) -> Result<Option<PathBuf>> {
        let profile_id = self.context.require_active_profile()?;
        Ok(CoverOf::thumbnail_shown_for(
            self.ports.artwork.as_ref(),
            profile_id,
            media_file_id,
        ))
    }

    /// The cover a tile draws for a track, if it has one.
    pub fn tile_for(&self, media_file_id: MediaFileId) -> Result<Option<PathBuf>> {
        let profile_id = self.context.require_active_profile()?;
        Ok(CoverOf::tile_shown_for(
            self.ports.artwork.as_ref(),
            profile_id,
            media_file_id,
        ))
    }

    /// Asks the listener for a picture and makes it this track's cover.
    ///
    /// Theirs and not the file's: the image is stored under the profile, the
    /// same way a corrected title is, so choosing a cover for yourself does not
    /// choose it for anybody else on the machine. The file on disk is never
    /// written to — Cadenza does not edit tags.
    ///
    /// `Ok(false)` means the chooser was closed, which is an answer.
    pub fn choose_cover(&self, media_file_id: MediaFileId) -> Result<bool> {
        let profile_id = self.context.require_active_profile()?;

        let chosen = cover::choose(
            &self.cover_ports(),
            CoverOf::ChosenTrack(profile_id, media_file_id),
            "Choose a cover",
        )?;
        if chosen {
            self.context.events.publish(DomainEvent::LibraryChanged);
        }
        Ok(chosen)
    }

    /// The three ports the shared picture flow needs, out of the ones this
    /// service already holds.
    pub(super) fn cover_ports(&self) -> cover::CoverPorts {
        cover::CoverPorts {
            artwork: Arc::clone(&self.ports.artwork),
            picker: Arc::clone(&self.ports.picker),
            files: Arc::clone(&self.ports.files),
        }
    }

    /// Takes back a chosen cover, leaving whatever the file itself carries.
    pub fn clear_cover(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .artwork
            .remove(CoverOf::ChosenTrack(profile_id, media_file_id))?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Caches embedded cover art, if the file had any.
    ///
    /// Failing to cache an image is not a reason to fail an import: the track is
    /// perfectly playable without a picture, and the alternative is a scan that
    /// stops because a disk is full of thumbnails.
    pub(super) fn cache_artwork(&self, media_file: &MediaFile, tags: &TrackTags) {
        if let Some(image) = &tags.artwork
            && let Err(err) = self
                .ports
                .artwork
                .store(CoverOf::Track(media_file.id), image)
        {
            self.context
                .warn(&format!("no cover was cached for {}: {err}", media_file.id));
        }
    }
}
