//! The folders the library is kept in: adding and watching them, scanning them, and what changed on disk.

use super::*;

impl LibraryService {
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

        // Already a folder of this profile's, and that is the answer rather
        // than a conflict: what the listener asked for is that this folder be
        // in their library, and it is. Pressing the offer twice used to reach
        // the unique index on `(profile_id, path)` and put its name in front of
        // somebody — "unique constraint failed" is not a sentence anybody
        // should be shown about a folder they can see.
        //
        // Re-enabled if it had been switched off, because pressing "use this
        // folder" about a folder that is switched off means switch it on.
        //
        // Compared as written, the way `local_folder` compares: two paths
        // differing only in case are the same folder to Windows and two rows
        // to SQLite, which is a smaller and separate wrong that nothing here
        // has hit yet.
        if let Some(mut existing) = self
            .folders()?
            .into_iter()
            .find(|folder| folder.path == path)
        {
            if !existing.enabled || existing.include_subfolders != include_subfolders {
                existing.enabled = true;
                existing.include_subfolders = include_subfolders;
                self.context.settings.save_folder(&existing)?;
            }
            self.watch(&existing)?;
            return Ok(existing);
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
        self.watch(&folder)?;
        Ok(folder)
    }

    /// Starts watching every enabled folder of the active profile.
    ///
    /// Called once by whoever supplied the watcher, after they have installed a
    /// handler for it. Folders added later are watched by [`Self::add_folder`],
    /// so this is about the folders that were already there.
    pub fn watch_folders(&self) -> Result<()> {
        for folder in self.folders()?.iter().filter(|folder| folder.enabled) {
            self.watch(folder)?;
        }
        Ok(())
    }

    /// Watches one folder, if there is a watcher to do it.
    ///
    /// The folder is saved before this runs, so a failure here means the library
    /// has the folder and will not hear about changes to it until the next
    /// start. That is worth reporting rather than hiding: it is the difference
    /// between a library that keeps itself current and one that does not.
    pub(super) fn watch(&self, folder: &ProfileFolder) -> Result<()> {
        match self.ports.watcher.as_ref() {
            Some(watcher) => watcher.watch(&folder.path, folder.include_subfolders),
            None => Ok(()),
        }
    }

    /// Asks the listener for a folder, adds it and scans it.
    ///
    /// One act rather than three. Somebody choosing a folder is saying "here is
    /// my music"; making them find a separate scan afterwards is asking them to
    /// say it twice.
    ///
    /// `None` means they closed the chooser, which is an answer.
    pub fn choose_folder(&self) -> Result<Option<ScanReport>> {
        let Some(path) = self
            .ports
            .picker
            .pick_folder("Choose a folder with your music")?
        else {
            return Ok(None);
        };

        let folder = self.add_folder(&path, true)?;
        self.adopt_folder(&folder).map(Some)
    }

    /// Where this machine keeps music, with a room of ours inside it.
    ///
    /// A suggestion and nothing more: nothing is created until
    /// [`Self::use_suggested_folder`] is called.
    pub fn suggested_folder(&self) -> Option<PathBuf> {
        self.ports.picker.suggested_music_folder()
    }

    /// Creates the suggested folder and starts watching it.
    ///
    /// The answer for somebody with no library and nowhere to point at. It
    /// writes to their filesystem, which is why it happens on a press rather
    /// than on a first run: a player that makes folders while nobody is looking
    /// is a player that has to be forgiven for it later.
    pub fn use_suggested_folder(&self) -> Result<ScanReport> {
        let path = self.suggested_folder().ok_or_else(|| {
            CoreError::invalid("library folder", "this machine has no music folder")
        })?;

        self.ports.files.create_dir_all(&path)?;
        let folder = self.add_folder(&path, true)?;
        self.adopt_folder(&folder)
    }

    /// The folder a fetched track lands in, if this listener has accepted one.
    ///
    /// Cadenza's own folder rather than "the first folder in the list": the
    /// other folders are places the listener pointed at, full of files they
    /// arranged themselves, and writing into somebody's collection because it
    /// happened to be first is how a player earns a reputation. What Cadenza
    /// puts on a disk goes in the room Cadenza was given.
    pub fn local_folder(&self) -> Result<Option<PathBuf>> {
        let Some(suggested) = self.suggested_folder() else {
            return Ok(None);
        };

        Ok(self
            .folders()?
            .into_iter()
            .find(|folder| folder.path == suggested)
            .map(|folder| folder.path))
    }

    /// What dropping files and folders onto the window means.
    ///
    /// A folder is an offer of somewhere to keep looking: it joins the library
    /// and is scanned, which is what choosing one through the chooser does. A
    /// file is an offer of one recording, and it is taken where it lies — a
    /// listener dragging in a single track is not asking for everything else in
    /// the directory it happened to be in.
    ///
    /// Anything else — a text file, a picture, a path that vanished between the
    /// drop and this call — is passed over in silence. A drop is a gesture with
    /// no undo and often no aim; refusing the whole handful because one of them
    /// was a cover image would be the wrong lesson to teach about it.
    pub fn accept_drop(&self, paths: &[PathBuf]) -> Result<ScanReport> {
        let profile_id = self.context.require_active_profile()?;
        let mut total = ScanReport::default();

        for path in paths {
            let Ok(metadata) = self.ports.files.metadata(path) else {
                continue;
            };

            if metadata.is_dir {
                let folder = self.add_folder(path, true)?;
                total.absorb(self.adopt_folder(&folder)?);
                continue;
            }

            if !has_supported_extension(path) {
                continue;
            }

            total.seen += 1;

            // Revived rather than merely imported, for the same reason
            // `adopt_folder` revives: dragging a file in is a newer decision
            // about it than having taken it out once was.
            match self.import_file(profile_id, path, metadata.size, metadata.modified, true) {
                Ok(Imported::Added) => total.added += 1,
                Ok(Imported::Updated) => total.updated += 1,
                Ok(Imported::Unchanged) => total.unchanged += 1,
                Ok(Imported::Duplicate) => total.duplicates += 1,
                Err(err) => {
                    total.failed += 1;
                    self.record_failure(profile_id, path, &err)?;
                }
            }
        }

        if total.added + total.updated + total.duplicates > 0 {
            self.context.events.publish(DomainEvent::LibraryChanged);
        }

        Ok(total)
    }

    /// Stops scanning a folder. Files already imported stay in the library.
    pub fn remove_folder(&self, folder: &ProfileFolder) -> Result<()> {
        self.context.settings.delete_folder(folder)?;
        if let Some(watcher) = self.ports.watcher.as_ref() {
            watcher.unwatch(&folder.path)?;
        }
        Ok(())
    }

    /// Scans every enabled folder of the active profile.
    ///
    /// And then looks for what the scan could not: a scan only ever meets files
    /// that exist, so a deletion made while Cadenza was closed is invisible to
    /// it. `refresh_missing` is the pass written for that, and until now
    /// nothing called it at all.
    pub fn scan_all(&self) -> Result<ScanReport> {
        let mut total = ScanReport::default();
        for folder in self.folders()?.iter().filter(|folder| folder.enabled) {
            total.absorb(self.scan_folder(folder)?);
        }
        self.refresh_missing()?;
        Ok(total)
    }

    /// Scans one folder.
    ///
    /// A file that cannot be read does not stop the scan: it becomes an entry in
    /// the review queue and the walk continues. One corrupt download must not
    /// cost the listener the other four thousand tracks.
    pub fn scan_folder(&self, folder: &ProfileFolder) -> Result<ScanReport> {
        self.walk(folder, false)
    }

    /// Scans a folder and brings back anything in it the listener had hidden.
    ///
    /// What adding a folder does, as against what a routine scan does. Taking
    /// one track out of the library is a decision, and a scan every few minutes
    /// must not undo it — but choosing the folder again is a newer decision
    /// about the same music, and the older one gives way to it.
    pub fn adopt_folder(&self, folder: &ProfileFolder) -> Result<ScanReport> {
        self.walk(folder, true)
    }

    pub(super) fn walk(&self, folder: &ProfileFolder, revive: bool) -> Result<ScanReport> {
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
                match self.import_file(profile_id, &entry, metadata.size, metadata.modified, revive)
                {
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

                match self.import_file(profile_id, path, metadata.size, metadata.modified, false) {
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

    /// Makes the library match the folder.
    ///
    /// Everything the folder holds is in the library, including what was taken
    /// out of it; everything the library holds from that folder and the folder
    /// no longer has is taken out. The file on disk is never touched.
    pub fn synchronise(&self, folder: &ProfileFolder) -> Result<ScanReport> {
        let profile_id = self.context.require_active_profile()?;
        let mut report = self.adopt_folder(folder)?;
        self.refresh_missing()?;

        let now = self.context.now();
        for summary in self.ports.tracks.summaries_for_profile(profile_id)? {
            if self.lies_under(folder, summary.media_file_id)
                && !self.still_there(summary.media_file_id)
            {
                self.ports
                    .tracks
                    .remove(profile_id, summary.media_file_id, now)?;
                report.gone += 1;
            }
        }

        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(report)
    }

    /// How many of this profile's tracks came from outside every folder.
    ///
    /// A file dropped on the window is taken where it lies, which means the
    /// library can hold tracks no folder is responsible for. They are not a
    /// mistake — they are simply not covered by scanning, watching or
    /// synchronising, and the settings screen says how many there are rather
    /// than leaving it to be discovered.
    pub fn outside_folders(&self) -> Result<usize> {
        let profile_id = self.context.require_active_profile()?;
        let folders = self.folders()?;

        Ok(self
            .ports
            .tracks
            .summaries_for_profile(profile_id)?
            .into_iter()
            .filter(|summary| {
                !folders
                    .iter()
                    .any(|folder| self.lies_under(folder, summary.media_file_id))
            })
            .count())
    }

    /// Whether a track's file is inside this folder.
    pub(super) fn lies_under(&self, folder: &ProfileFolder, media_file_id: MediaFileId) -> bool {
        let Ok(Some(file)) = self.ports.media_files.get(media_file_id) else {
            return false;
        };

        let Ok(rest) = file.path.strip_prefix(&folder.path) else {
            return false;
        };

        // Directly inside, or deeper when the folder was added with its
        // subfolders — the same question `walk` answers when it decides where
        // to look.
        folder.include_subfolders || rest.components().count() == 1
    }

    /// Whether the file behind a track is still on disk.
    ///
    /// Asked by the settings screen of everything that was taken out: a track
    /// whose file has gone cannot be brought back, and a button that says it
    /// can is a button that lies.
    pub fn is_on_disk(&self, media_file_id: MediaFileId) -> bool {
        self.still_there(media_file_id)
    }

    /// Whether the file behind a track is still on disk.
    pub(super) fn still_there(&self, media_file_id: MediaFileId) -> bool {
        self.ports
            .media_files
            .get(media_file_id)
            .ok()
            .flatten()
            .is_some_and(|file| self.ports.files.exists(&file.path))
    }
}
