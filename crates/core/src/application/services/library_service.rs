//! Scanning folders and importing what is found.
//!
//! The decisions live here — is this file new, changed, a duplicate, or a
//! problem — while the mechanisms (walking directories, reading tags, hashing)
//! sit behind ports in the infrastructure layer.

use std::path::Path;
use std::sync::Arc;

use crate::application::context::AppContext;
use crate::domain::album::Album;
use crate::domain::artist::Artist;
use crate::domain::genre::Genre;
use crate::domain::ids::{
    AlbumId, ArtistId, GenreId, ImportReviewId, MediaFileId, ProfileFolderId, ProfileId,
};
use crate::domain::media_file::{FileState, MediaFile, is_supported_extension};
use crate::domain::policies::duplicate_policy::{self, DuplicateVerdict};
use crate::domain::ports::artwork_cache::ArtworkCachePort;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::file_system::FileSystemPort;
use crate::domain::ports::file_watcher::FileChange;
use crate::domain::ports::metadata_reader::{FileMetadata, MetadataReaderPort, TrackTags};
use crate::domain::ports::repositories::{
    AlbumRepositoryPort, ArtistRepositoryPort, GenreRepositoryPort, ImportReviewRepositoryPort,
    MediaFileRepositoryPort, TrackRepositoryPort,
};
use crate::domain::review::{ImportReview, ReviewReason, ReviewResolution, ReviewState};
use crate::domain::settings::ProfileFolder;
use crate::domain::track::Track;
use crate::domain::value_objects::Timestamp;
use crate::{CoreError, Result};

/// Everything the library service talks to.
///
/// Bundled into one struct rather than nine constructor arguments: a call with
/// nine `Arc`s in a row is a place where two of them get swapped and nothing
/// complains until a test fails somewhere else entirely.
pub struct LibraryPorts {
    /// Reading directories, stat and hashing.
    pub files: Arc<dyn FileSystemPort>,
    /// Reading tags and stream properties.
    pub metadata: Arc<dyn MetadataReaderPort>,
    /// Cover art storage.
    pub artwork: Arc<dyn ArtworkCachePort>,
    /// The global file catalogue.
    pub media_files: Arc<dyn MediaFileRepositoryPort>,
    /// Per-profile library membership.
    pub tracks: Arc<dyn TrackRepositoryPort>,
    /// The artist catalogue.
    pub artists: Arc<dyn ArtistRepositoryPort>,
    /// The album catalogue.
    pub albums: Arc<dyn AlbumRepositoryPort>,
    /// The genre vocabulary.
    pub genres: Arc<dyn GenreRepositoryPort>,
    /// The import review queue.
    pub reviews: Arc<dyn ImportReviewRepositoryPort>,
}

/// What one scan did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Files with a supported extension that were looked at.
    pub seen: usize,
    /// Tracks added to the profile's library.
    pub added: usize,
    /// Files already known whose contents had changed.
    pub updated: usize,
    /// Files already known and unchanged.
    pub unchanged: usize,
    /// Files held back because they duplicate something already catalogued.
    pub duplicates: usize,
    /// Files that could not be read, each with an entry in the review queue.
    pub failed: usize,
}

/// What importing one file did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Imported {
    Added,
    Updated,
    Unchanged,
    Duplicate,
}

/// Scanning and import.
pub struct LibraryService {
    context: Arc<AppContext>,
    ports: LibraryPorts,
}

impl LibraryService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: LibraryPorts) -> Self {
        Self { context, ports }
    }

    /// The folders the active profile scans.
    pub fn folders(&self) -> Result<Vec<ProfileFolder>> {
        let profile_id = self.context.require_active_profile()?;
        self.context.settings.list_folders(profile_id)
    }

    /// Adds a folder to the active profile's library.
    ///
    /// The path must exist and be a directory. Accepting a typo and reporting
    /// "0 tracks found" later is the kind of failure people spend an evening on.
    pub fn add_folder(&self, path: &Path, include_subfolders: bool) -> Result<ProfileFolder> {
        let profile_id = self.context.require_active_profile()?;

        let metadata = self.ports.files.metadata(path)?;
        if !metadata.is_dir {
            return Err(CoreError::invalid(
                "library folder",
                format!("{} is not a directory", path.display()),
            ));
        }

        let folder = ProfileFolder {
            id: ProfileFolderId::new(),
            profile_id,
            path: path.to_path_buf(),
            include_subfolders,
            enabled: true,
            last_scan_at: None,
        };
        self.context.settings.save_folder(&folder)?;
        Ok(folder)
    }

    /// Stops scanning a folder. Files already imported stay in the library.
    pub fn remove_folder(&self, folder: &ProfileFolder) -> Result<()> {
        self.context.settings.delete_folder(folder)
    }

    /// Scans every enabled folder of the active profile.
    pub fn scan_all(&self) -> Result<ScanReport> {
        let mut total = ScanReport::default();
        for folder in self.folders()?.iter().filter(|folder| folder.enabled) {
            let report = self.scan_folder(folder)?;
            total.seen += report.seen;
            total.added += report.added;
            total.updated += report.updated;
            total.unchanged += report.unchanged;
            total.duplicates += report.duplicates;
            total.failed += report.failed;
        }
        Ok(total)
    }

    /// Scans one folder.
    ///
    /// A file that cannot be read does not stop the scan: it becomes an entry in
    /// the review queue and the walk continues. One corrupt download must not
    /// cost the listener the other four thousand tracks.
    pub fn scan_folder(&self, folder: &ProfileFolder) -> Result<ScanReport> {
        let profile_id = self.context.require_active_profile()?;
        if folder.profile_id != profile_id {
            return Err(CoreError::invalid(
                "library folder",
                "belongs to a different profile",
            ));
        }

        let mut report = ScanReport::default();
        let mut pending = vec![folder.path.clone()];

        while let Some(directory) = pending.pop() {
            // A directory that vanished mid-scan is not a scan failure. Removable
            // drives and cloud folders do this routinely.
            let Ok(entries) = self.ports.files.list_dir(&directory) else {
                continue;
            };

            for entry in entries {
                let Ok(metadata) = self.ports.files.metadata(&entry) else {
                    continue;
                };

                if metadata.is_dir {
                    if folder.include_subfolders {
                        pending.push(entry);
                    }
                    continue;
                }

                if !has_supported_extension(&entry) {
                    continue;
                }

                report.seen += 1;
                match self.import_file(profile_id, &entry, metadata.size, metadata.modified) {
                    Ok(Imported::Added) => report.added += 1,
                    Ok(Imported::Updated) => report.updated += 1,
                    Ok(Imported::Unchanged) => report.unchanged += 1,
                    Ok(Imported::Duplicate) => report.duplicates += 1,
                    Err(err) => {
                        report.failed += 1;
                        self.record_failure(profile_id, &entry, &err)?;
                    }
                }
            }
        }

        let scanned = ProfileFolder {
            last_scan_at: Some(self.context.now()),
            ..folder.clone()
        };
        self.context.settings.save_folder(&scanned)?;
        self.context.events.publish(DomainEvent::LibraryChanged);

        Ok(report)
    }

    /// Marks catalogued files that are no longer on disk, and unmarks any that
    /// came back. Returns how many rows changed.
    ///
    /// A scan only ever meets files that exist, so on its own it can never
    /// notice a deletion. Running this after a scan is what closes that gap for
    /// changes made while Cadenza was not running; the watcher covers the rest.
    ///
    /// ponytail: one catalogue lookup per track in the library. At the five
    /// thousand tracks the requirements name that is fine for something run once
    /// after a scan. If it ever runs per keystroke it wants a single query
    /// joining the two tables.
    pub fn refresh_missing(&self) -> Result<usize> {
        let profile_id = self.context.require_active_profile()?;
        let now = self.context.now();
        let mut changed = 0;

        for track in self.ports.tracks.list_for_profile(profile_id)? {
            let Some(file) = self.ports.media_files.get(track.media_file_id)? else {
                continue;
            };

            let present = self.ports.files.exists(&file.path);
            let should_be = if present {
                FileState::Available
            } else {
                FileState::Missing
            };

            // Only touch rows whose verdict actually changed, so a library of
            // five thousand healthy files costs five thousand reads and no
            // writes at all.
            if file.state != should_be {
                self.ports.media_files.set_state(file.id, should_be, now)?;
                changed += 1;
            }
        }

        if changed > 0 {
            self.context.events.publish(DomainEvent::LibraryChanged);
        }
        Ok(changed)
    }

    /// Applies one change reported by the filesystem watcher.
    ///
    /// Deliberately tolerant: a change concerning a file Cadenza never
    /// catalogued, or one with an extension it does not handle, is simply
    /// nothing to do. The watcher reports everything under a watched folder,
    /// including the listener's cover art and text files.
    pub fn apply_change(&self, change: &FileChange) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let now = self.context.now();

        match change {
            FileChange::Created(path) | FileChange::Modified(path) => {
                if !has_supported_extension(path) {
                    return Ok(());
                }
                let Ok(metadata) = self.ports.files.metadata(path) else {
                    // It went away again between the event and now. The removal
                    // event that follows will deal with it.
                    return Ok(());
                };
                if metadata.is_dir {
                    return Ok(());
                }

                match self.import_file(profile_id, path, metadata.size, metadata.modified) {
                    Ok(_) => {}
                    Err(err) => self.record_failure(profile_id, path, &err)?,
                }
            }

            FileChange::Removed(path) => {
                let Some(file) = self.ports.media_files.find_by_path(path)? else {
                    return Ok(());
                };
                // The catalogue row survives the file. It carries the listening
                // history and the playlist entries, and the file may well be
                // back in a moment — a rename often arrives as a removal
                // followed by a creation.
                self.ports
                    .media_files
                    .set_state(file.id, FileState::Missing, now)?;
            }

            FileChange::Renamed { from, to } => {
                let Some(file) = self.ports.media_files.find_by_path(from)? else {
                    // Not something we knew about; treat the destination as new.
                    return self.apply_change(&FileChange::Created(to.clone()));
                };
                self.ports.media_files.set_path(file.id, to, now)?;
            }
        }

        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Everything currently in the active profile's library.
    pub fn tracks(&self) -> Result<Vec<Track>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.tracks.list_for_profile(profile_id)
    }

    /// Removes a track from the active profile's library.
    ///
    /// The file stays on disk and in the catalogue, and other profiles keep
    /// their copy (PROJECT_MASTER 2.1).
    pub fn remove_track(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .tracks
            .remove(profile_id, media_file_id, self.context.now())?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Files waiting for a decision.
    pub fn pending_reviews(&self) -> Result<Vec<ImportReview>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)
    }

    /// Applies the listener's decision about a file held back for review.
    pub fn resolve_review(
        &self,
        review_id: ImportReviewId,
        resolution: ReviewResolution,
    ) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let now = self.context.now();

        let review = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .into_iter()
            .find(|entry| entry.id == review_id)
            .ok_or_else(|| CoreError::not_found("review entry", review_id))?;

        match resolution {
            // The new file is catalogued but never joins the library.
            ReviewResolution::KeepExisting => {}

            ReviewResolution::AddAnyway | ReviewResolution::EditMetadata => {
                self.add_to_library(profile_id, review.media_file_id, now)?;
            }

            ReviewResolution::RemoveExisting => {
                if let Some(existing) = review.duplicate_media_file_id {
                    self.ports.tracks.remove(profile_id, existing, now)?;
                }
                self.add_to_library(profile_id, review.media_file_id, now)?;
            }
        }

        self.ports
            .reviews
            .set_state(review_id, ReviewState::Resolved, now)?;
        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Imports one file, or reports why it could not be.
    fn import_file(
        &self,
        profile_id: ProfileId,
        path: &Path,
        size: u64,
        modified: Timestamp,
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
            return self.ensure_in_library(profile_id, file, now);
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
            // Held back, not discarded. The file is catalogued so a decision can
            // act on it, but it does not silently appear in the library
            // (PROJECT_MASTER 2.1).
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
    fn find_moved(&self, hash: &str) -> Result<Option<(MediaFileId, Timestamp)>> {
        for candidate in self.ports.media_files.find_by_hash(hash)? {
            if !self.ports.files.exists(&candidate.path) {
                return Ok(Some((candidate.id, candidate.created_at)));
            }
        }
        Ok(None)
    }

    /// True when this file already has an unresolved entry in the review queue.
    fn awaiting_decision(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<bool> {
        Ok(self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .iter()
            .any(|entry| entry.media_file_id == media_file_id))
    }

    /// Finds an already-catalogued file with the same contents at another path.
    ///
    /// The other copy has to still be there. The catalogue is global and outlives
    /// the profiles that referenced it, so it accumulates rows for files that
    /// have since been deleted or moved — and holding a new file back because it
    /// duplicates something that no longer exists is a decision the listener
    /// cannot even act on. A vanished row is marked missing on the way past,
    /// which is what `file_state` is for.
    fn find_duplicate(&self, candidate: &MediaFile, hash: &str) -> Result<Option<MediaFileId>> {
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
    fn upsert_track(
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

        let artist_id = self.resolve_artist(read.tags.artist.as_deref(), now)?;
        let album_artist_id = self.resolve_artist(
            read.tags
                .album_artist
                .as_deref()
                .or(read.tags.artist.as_deref()),
            now,
        )?;
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
            .unwrap_or_else(|| title_from_path(&media_file.path));

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
    fn ensure_in_library(
        &self,
        profile_id: ProfileId,
        media_file: &MediaFile,
        now: Timestamp,
    ) -> Result<Imported> {
        if self.ports.tracks.get(profile_id, media_file.id)?.is_some() {
            return Ok(Imported::Unchanged);
        }

        let track = Track {
            profile_id,
            media_file_id: media_file.id,
            title: title_from_path(&media_file.path),
            artist_id: None,
            album_id: None,
            track_no: None,
            disc_no: None,
            year: None,
            added_at: now,
            removed_at: None,
        };
        self.ports.tracks.save(&track)?;
        Ok(Imported::Added)
    }

    /// Adds a catalogued file to a profile's library by identifier.
    fn add_to_library(
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

        self.ensure_in_library(profile_id, &media_file, now)
            .map(|_| ())
    }

    /// Finds or creates an artist by name.
    fn resolve_artist(&self, name: Option<&str>, now: Timestamp) -> Result<Option<ArtistId>> {
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
    fn resolve_album(
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
    fn link_genres(&self, media_file_id: MediaFileId, names: &[String]) -> Result<()> {
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

        self.ports.genres.set_for_media_file(media_file_id, &ids)
    }

    /// Caches embedded cover art, if the file had any.
    ///
    /// Failing to cache an image is not a reason to fail an import: the track is
    /// perfectly playable without a picture, and the alternative is a scan that
    /// stops because a disk is full of thumbnails.
    fn cache_artwork(&self, media_file: &MediaFile, tags: &TrackTags) {
        if let Some(image) = &tags.artwork {
            let _ = self.ports.artwork.store(media_file.id, image);
        }
    }

    /// Turns a failed import into a review entry.
    fn record_failure(&self, profile_id: ProfileId, path: &Path, err: &CoreError) -> Result<()> {
        let reason = match err {
            CoreError::Metadata(_) => ReviewReason::UnreadableMetadata,
            CoreError::Decode(_) => ReviewReason::UndecodableAudio,
            _ => ReviewReason::MissingFile,
        };

        // The entry needs a catalogue row to point at. A file that could not be
        // read far enough to be catalogued has nothing to attach a decision to,
        // so it is counted as a failure and left for the next scan.
        let Some(media_file) = self.ports.media_files.find_by_path(path)? else {
            return Ok(());
        };

        self.raise_review(profile_id, media_file.id, None, reason, self.context.now())
    }

    /// Puts a file in front of the listener.
    fn raise_review(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        duplicate_of: Option<MediaFileId>,
        reason: ReviewReason,
        now: Timestamp,
    ) -> Result<()> {
        // Rescanning the same unresolved problem must not stack up entries.
        let already_pending = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?
            .into_iter()
            .any(|entry| entry.media_file_id == media_file_id && entry.reason == reason);
        if already_pending {
            return Ok(());
        }

        let review = ImportReview {
            id: ImportReviewId::new(),
            profile_id,
            media_file_id,
            duplicate_media_file_id: duplicate_of,
            reason,
            state: ReviewState::Pending,
            created_at: now,
            resolved_at: None,
        };
        self.ports.reviews.save(&review)?;
        self.context.events.publish(DomainEvent::ReviewPending);
        Ok(())
    }
}

/// True when a path is worth opening.
fn has_supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(is_supported_extension)
}

/// The title to show for a file whose tags did not provide one.
///
/// The filename, not "Unknown": an untagged file usually has a name that says
/// exactly what it is, and a hundred rows of "Unknown" help nobody.
fn title_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Untitled");
    let collapsed = stem
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        "Untitled".to_owned()
    } else {
        collapsed
    }
}
