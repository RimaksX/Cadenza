//! Reading a file into the library: its tags, its artist and album, and what it duplicates.

use super::*;

impl LibraryService {
    /// Imports one file, or reports why it could not be.
    pub(super) fn import_file(
        &self,
        profile_id: ProfileId,
        path: &Path,
        size: u64,
        modified: Timestamp,
        revive: bool,
    ) -> Result<Imported> {
        let now = self.context.now();
        let known = self.ports.media_files.find_by_path(path)?;

        // Nothing about the file changed and the reader has not been improved
        // since it was last read, so there is nothing to re-read. This is the
        // path almost every file takes on almost every scan, which is why it
        // costs one stat and one indexed lookup rather than a full hash.
        if let Some(file) = &known
            && !file.is_stale(size, modified)
            && file.metadata_version.as_deref() == Some(self.ports.metadata.version())
        {
            // A file already waiting for a decision stays out of the library.
            // Without this the next scan takes the unchanged path, finds no
            // track row, and helpfully adds the very duplicate that is sitting
            // in the review queue — which would make the queue pointless.
            if self.awaiting_decision(profile_id, file.id)? {
                return Ok(Imported::Duplicate);
            }

            // Standing here is proof the file answered: something just read its
            // size and its modification time. A row still marked missing from
            // an earlier disappearance has to be corrected now, because nothing
            // else on this path writes the state — which is how a file that had
            // come back stayed unplayable through a scan, a synchronise and a
            // folder removed and added again.
            if !file.state.is_playable() {
                self.ports
                    .media_files
                    .set_state(file.id, FileState::Available, now)?;
            }

            return self.ensure_in_library(profile_id, file, now, revive);
        }

        let read = self.ports.metadata.read(path)?;
        let hash = self.ports.files.hash_file(path)?;

        // Content that matches a catalogued row whose file is gone is that file
        // in a new place, not a new file. Without this a rename leaves a phantom
        // entry pointing at nothing and a second one beside it — and on Windows
        // a rename is reported as a removal followed by a creation, so this is
        // the common case rather than the exotic one.
        let moved = match &known {
            Some(_) => None,
            None => self.find_moved(&hash)?,
        };
        if let Some((id, _)) = &moved {
            self.ports.media_files.set_path(*id, path, now)?;
        }

        let id = known
            .as_ref()
            .map(|file| file.id)
            .or_else(|| moved.as_ref().map(|(id, _)| *id))
            .unwrap_or_else(MediaFileId::new);
        let created_at = known
            .as_ref()
            .map(|file| file.created_at)
            .or_else(|| moved.as_ref().map(|(_, created_at)| *created_at))
            .unwrap_or(now);

        let media_file = MediaFile {
            id,
            path: path.to_path_buf(),
            file_hash: Some(hash.clone()),
            file_size: size,
            file_mtime: modified,
            format: read.format,
            properties: read.properties,
            metadata_version: Some(self.ports.metadata.version().to_owned()),
            metadata_extracted_at: Some(now),
            state: FileState::Available,
            created_at,
            updated_at: now,
        };
        self.ports.media_files.save(&media_file)?;
        self.cache_artwork(&media_file, &read.tags);

        if let Some(other) = self.find_duplicate(&media_file, &hash)? {
            // Held back, not discarded. The file is catalogued so a decision
            // can act on it, but it does not silently appear in the library.
            self.raise_review(
                profile_id,
                media_file.id,
                Some(other),
                ReviewReason::Duplicate,
                now,
            )?;
            return Ok(Imported::Duplicate);
        }

        self.upsert_track(profile_id, &media_file, &read, now)
    }

    /// Finds a catalogued row with this content whose file is no longer there.
    ///
    /// Returns its identifier and the moment it was first seen, both of which
    /// the moved file keeps: it is the same recording, and its listening history
    /// and playlist entries hang off that identifier.
    pub(super) fn find_moved(&self, hash: &str) -> Result<Option<(MediaFileId, Timestamp)>> {
        for candidate in self.ports.media_files.find_by_hash(hash)? {
            if !self.ports.files.exists(&candidate.path) {
                return Ok(Some((candidate.id, candidate.created_at)));
            }
        }
        Ok(None)
    }

    /// Finds an already-catalogued file with the same contents at another path.
    ///
    /// The other copy has to still be there. The catalogue is global and outlives
    /// the profiles that referenced it, so it accumulates rows for files that
    /// have since been deleted or moved — and holding a new file back because it
    /// duplicates something that no longer exists is a decision the listener
    /// cannot even act on. A vanished row is marked missing on the way past,
    /// which is what `file_state` is for.
    pub(super) fn find_duplicate(
        &self,
        candidate: &MediaFile,
        hash: &str,
    ) -> Result<Option<MediaFileId>> {
        for other in self.ports.media_files.find_by_hash(hash)? {
            if duplicate_policy::compare(candidate, &other) != DuplicateVerdict::SameContent {
                continue;
            }

            if self.ports.files.exists(&other.path) {
                return Ok(Some(other.id));
            }

            self.ports
                .media_files
                .set_state(other.id, FileState::Missing, self.context.now())?;
        }
        Ok(None)
    }

    /// Adds or refreshes the profile's row for a file.
    pub(super) fn upsert_track(
        &self,
        profile_id: ProfileId,
        media_file: &MediaFile,
        read: &FileMetadata,
        now: Timestamp,
    ) -> Result<Imported> {
        let existing = self.ports.tracks.get(profile_id, media_file.id)?;

        // A track the listener removed stays removed. Re-adding it on the next
        // scan would make "remove from library" meaningless for any file inside
        // a watched folder.
        if let Some(track) = &existing
            && track.removed_at.is_some()
        {
            return Ok(Imported::Unchanged);
        }

        // What the file is called, for the parts its tags do not carry.
        //
        // A track fetched from a link arrives with no tags on purpose — what
        // the video calls itself is not what the record is called — and the
        // name we gave it holds both facts. Every other untagged file in the
        // world is named the same way.
        let named = title_from_path(&media_file.path);
        let (named_artist, named_title) = naming_policy::artist_and_title(&named);

        let artist = read.tags.artist.as_deref().or(named_artist);
        let artist_id = self.resolve_artist(artist, now)?;
        let album_artist_id =
            self.resolve_artist(read.tags.album_artist.as_deref().or(artist), now)?;
        let album_id = self.resolve_album(
            read.tags.album.as_deref(),
            album_artist_id,
            read.tags.year,
            now,
        )?;
        self.link_genres(media_file.id, &read.tags.genres)?;

        let title = read
            .tags
            .title
            .clone()
            .unwrap_or_else(|| named_title.to_owned());

        let track = Track {
            profile_id,
            media_file_id: media_file.id,
            title,
            artist_id,
            album_id,
            track_no: read.tags.track_no,
            disc_no: read.tags.disc_no,
            year: read.tags.year,
            added_at: existing.as_ref().map_or(now, |track| track.added_at),
            removed_at: None,
        };
        self.ports.tracks.save(&track)?;

        Ok(if existing.is_some() {
            Imported::Updated
        } else {
            Imported::Added
        })
    }

    /// Makes sure an unchanged file is in this profile's library.
    ///
    /// Two profiles share the catalogue but not their libraries, so a file the
    /// machine already knows may still be new to this listener.
    pub(super) fn ensure_in_library(
        &self,
        profile_id: ProfileId,
        media_file: &MediaFile,
        now: Timestamp,
        revive: bool,
    ) -> Result<Imported> {
        if let Some(track) = self.ports.tracks.get(profile_id, media_file.id)? {
            if !revive || track.removed_at.is_none() {
                return Ok(Imported::Unchanged);
            }

            // Taken out of the library once, and now inside a folder the
            // listener has just pointed at again. Pointing at a folder is a
            // statement about everything in it, and it is the newer of the two.
            //
            // Without this there is no way back at all: the file is on disk, in
            // a watched folder, catalogued and unchanged, so every later scan
            // takes the fast path and leaves it hidden for good.
            let restored = Track {
                removed_at: None,
                ..track
            };
            self.ports.tracks.save(&restored)?;
            return Ok(Imported::Added);
        }

        // The file is catalogued, so this scan took the fast path and never
        // opened it. Read it now: a listener joining a file another profile
        // imported first is owed the same title, artist and album as they got.
        //
        // Copying the other profile's row instead would be cheaper and wrong —
        // it would hand over their corrections, which is the leak profiles
        // exist to prevent.
        let read = self.ports.metadata.read(&media_file.path)?;
        self.upsert_track(profile_id, media_file, &read, now)
    }

    /// Adds a catalogued file to a profile's library by identifier.
    pub(super) fn add_to_library(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        now: Timestamp,
    ) -> Result<()> {
        let media_file = self
            .ports
            .media_files
            .get(media_file_id)?
            .ok_or_else(|| CoreError::not_found("media file", media_file_id))?;

        if self.ports.tracks.get(profile_id, media_file_id)?.is_some() {
            return self.ports.tracks.restore(profile_id, media_file_id);
        }

        self.ensure_in_library(profile_id, &media_file, now, true)
            .map(|_| ())
    }

    /// Finds or creates an artist by name.
    pub(super) fn resolve_artist(
        &self,
        name: Option<&str>,
        now: Timestamp,
    ) -> Result<Option<ArtistId>> {
        let Some(name) = name else {
            return Ok(None);
        };

        if let Some(existing) = self.ports.artists.find_by_name(name)? {
            return Ok(Some(existing.id));
        }

        let artist = Artist {
            id: ArtistId::new(),
            name: name.to_owned(),
            sort_name: Artist::derive_sort_name(name),
            created_at: now,
            updated_at: now,
        };
        self.ports.artists.save(&artist)?;
        Ok(Some(artist.id))
    }

    /// Finds or creates an album by title and album artist.
    pub(super) fn resolve_album(
        &self,
        title: Option<&str>,
        artist_id: Option<ArtistId>,
        year: Option<u16>,
        now: Timestamp,
    ) -> Result<Option<AlbumId>> {
        let Some(title) = title else {
            return Ok(None);
        };

        if let Some(existing) = self.ports.albums.find(title, artist_id)? {
            return Ok(Some(existing.id));
        }

        let album = Album {
            id: AlbumId::new(),
            artist_id,
            title: title.to_owned(),
            year,
            created_at: now,
            updated_at: now,
        };
        self.ports.albums.save(&album)?;
        Ok(Some(album.id))
    }

    /// Attaches a file to its genres, creating any that are new.
    pub(super) fn link_genres(&self, media_file_id: MediaFileId, names: &[String]) -> Result<()> {
        let ids = self.genre_ids(names)?;
        self.ports.genres.set_for_media_file(media_file_id, &ids)
    }

    /// Turns genre names into identifiers, adding any the vocabulary lacks.
    ///
    /// Normalisation happens here rather than in the caller, so a genre typed by
    /// a listener and one read from a tag collapse onto the same row.
    pub(super) fn genre_ids(&self, names: &[String]) -> Result<Vec<GenreId>> {
        let mut ids: Vec<GenreId> = Vec::with_capacity(names.len());

        for name in names {
            let normalized = Genre::normalize(name);
            if normalized.is_empty() {
                continue;
            }

            let id = match self.ports.genres.find_by_name(&normalized)? {
                Some(existing) => existing.id,
                None => {
                    let genre = Genre {
                        id: GenreId::new(),
                        name: normalized,
                    };
                    self.ports.genres.save(&genre)?;
                    genre.id
                }
            };

            if !ids.contains(&id) {
                ids.push(id);
            }
        }

        Ok(ids)
    }

    /// Turns a failed import into a review entry.
    pub(super) fn record_failure(
        &self,
        profile_id: ProfileId,
        path: &Path,
        err: &CoreError,
    ) -> Result<()> {
        let Some(reason) = review_reason(err) else {
            self.context.warn(&format!(
                "{} was not imported, and it is not the file's fault: {err}",
                path.display()
            ));
            return Ok(());
        };

        // The entry needs a catalogue row to point at. A file that could not be
        // read far enough to be catalogued has nothing to attach a decision to,
        // so it is counted as a failure and left for the next scan.
        let Some(media_file) = self.ports.media_files.find_by_path(path)? else {
            return Ok(());
        };

        self.raise_review(profile_id, media_file.id, None, reason, self.context.now())
    }
}
