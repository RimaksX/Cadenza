//! Scanning folders and importing what is found.
//!
//! The decisions live here — is this file new, changed, a duplicate, or a
//! problem — while the mechanisms (walking directories, reading tags, hashing)
//! sit behind ports in the infrastructure layer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::cover;
use crate::application::context::AppContext;
use crate::domain::album::Album;
use crate::domain::artist::Artist;
use crate::domain::genre::Genre;
use crate::domain::ids::{
    AlbumId, ArtistId, GenreId, ImportReviewId, MediaFileId, ProfileFolderId, ProfileId,
};
use crate::domain::media_file::{FileState, MediaFile, is_supported_extension};
use crate::domain::policies::duplicate_policy::{self, DuplicateVerdict};
use crate::domain::policies::fetch_policy::looks_out_of_date;
use crate::domain::policies::link_policy::{LinkHandler, handler_for, is_a_link};
use crate::domain::policies::naming_policy;
use crate::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::fetcher::{
    FetchPort, FetchProgress, FetchWhat, ListedTrack, MissingTool,
};
use crate::domain::ports::file_system::FileSystemPort;
use crate::domain::ports::file_watcher::{FileChange, FileWatcherPort};
use crate::domain::ports::folder_picker::FolderPickerPort;
use crate::domain::ports::metadata_reader::{FileMetadata, MetadataReaderPort, TrackTags};
use crate::domain::ports::repositories::{
    AlbumRepositoryPort, ArtistRepositoryPort, GenreRepositoryPort, ImportReviewRepositoryPort,
    MediaFileRepositoryPort, TrackRepositoryPort,
};
use crate::domain::review::{ImportReview, ReviewReason, ReviewResolution, ReviewState};
use crate::domain::settings::ProfileFolder;
use crate::domain::track::{Track, TrackSummary};
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
    /// Where a playlist brought in from a link becomes a playlist here.
    ///
    /// The service rather than the repository, because "make a playlist" has
    /// rules — a name has to be valid and unique to the profile — and they are
    /// written down once, there. Optional like the watcher: a command that
    /// scans a folder has no playlists to make.
    pub playlists: Option<Arc<super::PlaylistService>>,
    /// The system's folder chooser, and its opinion about where music lives.
    pub picker: Arc<dyn FolderPickerPort>,
    /// The watcher that keeps the library current, when there is one.
    ///
    /// Optional because every test that is not about watching should not have
    /// to provide one.
    pub watcher: Option<Arc<dyn FileWatcherPort>>,
    /// Somebody else's downloader, when this machine has one.
    ///
    /// Optional for the same reason as the watcher, and for one more: this is
    /// the only part of Cadenza that touches a network at all, and a context
    /// built without it is a context that provably cannot.
    pub fetcher: Option<Arc<dyn FetchPort>>,
}

/// What one scan did.
///
/// Counts rather than a list: a scan of five thousand files that reported each
/// one would be a report nobody reads.
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
    /// Tracks taken out because the folder no longer holds their file.
    ///
    /// Only synchronising fills this in: a routine scan leaves a track whose
    /// file has gone where it is, because a disconnected drive is not a
    /// decision to forget an album.
    pub gone: usize,
}

/// How a pasted link ended.
///
/// Three outcomes rather than a result and two error strings, because two of
/// these are things the listener can *do* something about and the window has to
/// offer them the doing. An error the interface has to read the words of to
/// know which button to show is an error that will be read wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    /// It is in the library, under this name.
    Landed(String),
    /// A playlist came in: this many tracks are in the library.
    ///
    /// A count rather than a list of names, because forty names is not a thing
    /// a line under a button can say — and the library below is already
    /// showing them.
    LandedMany(usize),
    /// Nothing came, and nothing went wrong.
    ///
    /// Either the listener stopped it, or every track in the playlist was
    /// already here — which is what a second press on the same link means once
    /// yt-dlp's own record of what it has fetched is doing its work.
    NothingNew,
    /// There is nowhere for it to land: this listener has no local folder.
    ///
    /// Carries the folder Cadenza would make, so the offer can name it.
    NeedsLocalFolder(PathBuf),
    /// The machine has not got what it takes to fetch anything.
    NeedsTools(Vec<MissingTool>),
    /// It failed in one of the ways a downloader that has fallen behind fails.
    ///
    /// Carries what it said, because that is still the truest thing anybody
    /// can be told — the offer to update is what is added to it, not what
    /// replaces it.
    NeedsUpdate(String),
}

/// What importing one file did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Imported {
    Added,
    Updated,
    Unchanged,
    Duplicate,
}

impl ScanReport {
    /// Adds another report to this one.
    fn absorb(&mut self, other: Self) {
        self.gone += other.gone;
        self.seen += other.seen;
        self.added += other.added;
        self.updated += other.updated;
        self.unchanged += other.unchanged;
        self.duplicates += other.duplicates;
        self.failed += other.failed;
    }
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

        // Already a folder of this profile's, and that is the answer rather
        // than a conflict: what the listener asked for is that this folder be
        // in their library, and it is. Pressing the offer twice used to reach
        // the unique index on `(profile_id, path)` and put its name in front
        // of somebody — "unique constraint failed" is not a sentence anybody
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
    fn watch(&self, folder: &ProfileFolder) -> Result<()> {
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

    /// Brings down whatever is at `link` and puts it in the library.
    ///
    /// Long: this runs a download and a conversion, so it belongs on a thread
    /// and never on the one drawing the window. `progress` is called with whole
    /// percentages as they arrive, on that same thread.
    ///
    /// The order of the three checks is the order the listener can act on them.
    /// Whether the link is a link is instant and theirs to fix; whether the
    /// tools exist is a five-minute install; whether there is a folder is one
    /// press. Discovering the third after waiting for a download would be a
    /// download thrown away.
    pub fn fetch_from_link(
        &self,
        link: &str,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> Result<Fetched> {
        let profile_id = self.context.require_active_profile()?;
        let link = link.trim();

        if !is_a_link(link) {
            return Err(CoreError::invalid(
                "link",
                "that is not a web address — paste the whole thing, starting with https://",
            ));
        }

        // Before the tools, because this is true whatever is installed: no
        // version of anything will ever fetch from these, and the listener's
        // next move is the same track somewhere that will part with it.
        //
        // Spotify used to be on that list and is not any more, and the
        // distinction is worth keeping straight: nothing can take Spotify's
        // *audio*, which is still true, but its links name a recording and the
        // recording can be found. That is what the matcher does, and what
        // every service claiming to "download from Spotify" does.
        if let LinkHandler::Refused(service) = handler_for(link) {
            return Err(CoreError::invalid(
                "link",
                format!(
                    "{service} encrypts its audio — nothing can fetch it.                      Find the track on YouTube instead"
                ),
            ));
        }

        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        let missing = fetcher.missing_for(link);
        if !missing.is_empty() {
            return Ok(Fetched::NeedsTools(missing));
        }

        let Some(folder) = self.local_folder()? else {
            // Not an error: the listener was asked once whether Cadenza could
            // make itself a folder and said no, which was a fair answer to a
            // question about their disk. Now there is a reason, so the offer
            // comes back with one.
            let would_be = self.suggested_folder().ok_or_else(|| {
                CoreError::invalid("local folder", "this machine has no music folder")
            })?;
            return Ok(Fetched::NeedsLocalFolder(would_be));
        };

        // What this listener already has, by the same rule the playlist uses
        // to find them: the tags a track carries, compared with the names the
        // list gives. Read once rather than per track.
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;
        let have = |track: &ListedTrack| {
            library
                .iter()
                .any(|summary| summary_is(summary, &track.title, &track.artist))
        };

        let brought = match fetcher.fetch(link, &folder, what, progress, stop, &have) {
            Ok(brought) => brought,
            // A refusal that reads like a stale copy is an offer rather than an
            // error: the listener can fix it by pressing one thing, and being
            // told so beats being told what went wrong.
            Err(CoreError::Invalid { field, reason }) if looks_out_of_date(&reason) => {
                debug_assert_eq!(field, "link");
                return Ok(Fetched::NeedsUpdate(reason));
            }
            Err(err) => return Err(err),
        };
        let files = brought.files;

        // From here they are ordinary files that appeared in a watched folder,
        // and they go through the same import as one somebody copied in — the
        // same tags, the same duplicate check, the same review queue when one
        // cannot be read. A track is a track however it arrived.
        for file in &files {
            let metadata = self.ports.files.metadata(file)?;
            self.import_file(profile_id, file, metadata.size, metadata.modified, true)?;
        }

        // A playlist that came in as a playlist becomes one here, under the
        // name it had where it came from. Forty tracks landing loose in a
        // library is forty tracks somebody has to gather up by hand — and the
        // thing they were part of is exactly what they pasted.
        //
        // Failing to make it is not failing to fetch: the tracks are in the
        // library either way, and that is what was asked for.
        if let Some(name) = brought.playlist.as_deref()
            && let Err(err) = self.gather_into_playlist(name, &files, &brought.listed)
        {
            self.context.warn(&format!(
                "the tracks came in but the playlist did not: {err}"
            ));
        }

        // Nothing new on the disk, and that is not nothing done: the list
        // above was still rebuilt from what the listener already has, which is
        // the whole point of pressing it a second time.
        if files.is_empty() {
            return Ok(Fetched::NothingNew);
        }
        self.context.events.publish(DomainEvent::LibraryChanged);

        // One track is named; forty are counted. Naming the first of forty
        // would be a line that answers a question nobody asked.
        if let (FetchWhat::OneTrack, Some(file)) = (what, files.first()) {
            return Ok(Fetched::Landed(
                file.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }

        Ok(Fetched::LandedMany(files.len()))
    }

    /// Puts what just arrived into a playlist of that name, making it if it is
    /// new and adding to it if it is not.
    ///
    /// Adding rather than refusing, because that is what a second press means:
    /// a playlist fetched again brings whatever was added to it since, and
    /// those tracks belong with the ones already here. A track already in the
    /// playlist is not added twice — `add_track` is what decides that.
    fn gather_into_playlist(
        &self,
        name: &str,
        files: &[PathBuf],
        listed: &[ListedTrack],
    ) -> Result<()> {
        let Some(playlists) = self.ports.playlists.as_ref() else {
            return Ok(());
        };

        let existing = playlists
            .list()?
            .into_iter()
            .map(|summary| summary.playlist)
            .find(|playlist| playlist.name.as_str().eq_ignore_ascii_case(name));

        let playlist = match existing {
            Some(playlist) => playlist,
            None => playlists.create(name)?,
        };

        // Everything the list names that this listener has, whether it came
        // just now or a week ago.
        //
        // A playlist holds the *list*. A second fetch of the same address
        // fetches almost nothing — the memory sees to that — so a playlist
        // built from what arrived would hold the two tracks that happened to
        // be new and none of the fifty that were already here.
        //
        // Matched on title and artist rather than on a file name: both ends of
        // that comparison came from the same metadata — the matcher wrote the
        // tags, the library read them back — while a file name has been through
        // one program's idea of what a filename may contain.
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        // What it already holds counts as added, or fetching the same list
        // twice would put every track in it twice — and `add_track` appends
        // whatever it is given, as it should: it is the caller who knows
        // whether this is the same list coming round again.
        let mut added: std::collections::HashSet<_> = playlists
            .tracks_of(playlist.id)?
            .into_iter()
            .map(|track| track.media_file_id)
            .collect();

        for track in listed {
            let found = library
                .iter()
                .find(|summary| summary_is(summary, &track.title, &track.artist));

            if let Some(summary) = found
                && added.insert(summary.media_file_id)
                && playlists
                    .add_track(playlist.id, summary.media_file_id)
                    .is_err()
            {
                added.remove(&summary.media_file_id);
            }
        }

        // One track that will not join must not cost the other fifty-one.
        //
        // It used to. A file the import set aside — a duplicate of one already
        // in the library, which is what a second fetch of the same list is full
        // of — is not in the library, `add_track` says so, and the `?` here
        // threw away every track after it. A listener watched three new tracks
        // arrive and the playlist stay exactly where it was.
        //
        // And whatever arrived that the list did not name, or named
        // differently: the tracks a YouTube link brought, and any whose tags
        // the listener has since corrected.
        let mut missed = 0;
        for file in files {
            let joined = match self.ports.media_files.find_by_path(file)? {
                Some(media_file) => {
                    !added.insert(media_file.id)
                        || playlists.add_track(playlist.id, media_file.id).is_ok()
                }
                None => false,
            };
            if !joined {
                missed += 1;
            }
        }

        if missed > 0 {
            self.context.warn(&format!(
                "{missed} of {} did not join the playlist: they are already in                  the library, or waiting for a decision",
                files.len()
            ));
        }

        Ok(())
    }

    /// Installs the programs a fetch needs, through the machine's own package
    /// manager, and says what is still missing afterwards.
    ///
    /// Empty means it worked. `said` is called as it goes, because installing
    /// something on somebody's computer is not a thing to do behind a spinner.
    pub fn install_tools(&self, link: &str, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>> {
        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        fetcher.install(link, said)
    }

    /// Brings the downloader up to date, and hands back what it said about it.
    pub fn update_downloader(&self, said: &dyn Fn(&str)) -> Result<String> {
        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        fetcher.update(said)
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

    /// Asks the listener for a picture and makes it this track's cover.
    ///
    /// Theirs and not the file's: the image is stored under the profile, the
    /// same way a corrected title is, so choosing a cover for yourself does not
    /// choose it for anybody else on the machine.
    /// The file on disk is never written to — Cadenza does not edit tags.
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
    fn cover_ports(&self) -> cover::CoverPorts {
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

    fn walk(&self, folder: &ProfileFolder, revive: bool) -> Result<ScanReport> {
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
    /// **Only the ones with no file left, and that is the whole design.** A
    /// tombstone is two things at once: an offer to undo, and the record that
    /// keeps a removed track out when its folder is scanned again. Where the
    /// file is gone it is neither — nothing can be brought back to it and no
    /// scan will ever find it — so it is only a row nobody can act on, and a
    /// listener who deleted a folder of fifty-two tracks is left with
    /// fifty-two of them. Where the file is still there the row is doing its
    /// job, and forgetting it would put the track back at the next scan.
    ///
    /// Never automatic, for the same reason. A folder on a drive that is
    /// unplugged looks exactly like a folder that was deleted, and the
    /// difference shows up when the drive comes back.
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
    fn lies_under(&self, folder: &ProfileFolder, media_file_id: MediaFileId) -> bool {
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
    fn still_there(&self, media_file_id: MediaFileId) -> bool {
        self.ports
            .media_files
            .get(media_file_id)
            .ok()
            .flatten()
            .is_some_and(|file| self.ports.files.exists(&file.path))
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

    /// Files waiting for a decision.
    pub fn pending_reviews(&self) -> Result<Vec<ImportReview>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)
    }

    /// The waiting decisions, with enough about each to make it.
    ///
    /// A row of the review queue names a file the listener has never seen: it
    /// was held back before it reached the library. What they need is where it
    /// is, why it is waiting, and — for a duplicate — what it is a duplicate
    /// *of*, which is a track they do know.
    pub fn review_cards(&self) -> Result<Vec<ReviewCard>> {
        let profile_id = self.context.require_active_profile()?;
        let pending = self
            .ports
            .reviews
            .list_for_profile(profile_id, ReviewState::Pending)?;

        let mut cards = Vec::with_capacity(pending.len());
        for review in pending {
            let path = self
                .ports
                .media_files
                .get(review.media_file_id)?
                .map(|file| file.path);

            let existing = match review.duplicate_media_file_id {
                Some(id) => self.ports.tracks.summary(profile_id, id)?,
                None => None,
            };

            cards.push(ReviewCard {
                id: review.id,
                reason: review.reason,
                path,
                existing,
            });
        }
        Ok(cards)
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
            // Held back, not discarded. The file is catalogued so a decision can
            // act on it, but it does not silently appear in the library.
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

        // What the file is called, for the parts its tags do not carry.
        //
        // A track fetched from a link arrives with no tags on purpose — what
        // the video calls itself is not what the record is called — and the
        // name we gave it holds both facts. Every other
        // untagged file in the world is named the same way.
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
    fn ensure_in_library(
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
        // it would hand over their corrections, which is the leak profiles exist to prevent.
        let read = self.ports.metadata.read(&media_file.path)?;
        self.upsert_track(profile_id, media_file, &read, now)
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

        self.ensure_in_library(profile_id, &media_file, now, true)
            .map(|_| ())
    }

    /// Corrects what this profile calls a track.
    ///
    /// A local override and nothing else: the file keeps its tags, the
    /// catalogue keeps its reading of them, and another profile sharing the
    /// same file goes on seeing what it always saw. There
    /// is no writing back to disk and there is not meant to be.
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
        let ids = self.genre_ids(names)?;
        self.ports.genres.set_for_media_file(media_file_id, &ids)
    }

    /// Turns genre names into identifiers, adding any the vocabulary lacks.
    ///
    /// Normalisation happens here rather than in the caller, so a genre typed by
    /// a listener and one read from a tag collapse onto the same row.
    fn genre_ids(&self, names: &[String]) -> Result<Vec<GenreId>> {
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

    /// Caches embedded cover art, if the file had any.
    ///
    /// Failing to cache an image is not a reason to fail an import: the track is
    /// perfectly playable without a picture, and the alternative is a scan that
    /// stops because a disk is full of thumbnails.
    fn cache_artwork(&self, media_file: &MediaFile, tags: &TrackTags) {
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

    /// Turns a failed import into a review entry.
    fn record_failure(&self, profile_id: ProfileId, path: &Path, err: &CoreError) -> Result<()> {
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

/// Whether a library row is the recording a list names.
///
/// Title and artist, both ignoring case. Written once because two places ask
/// it — what to skip fetching, and what to put in the playlist — and they must
/// never disagree: a track skipped as already here and then not found for the
/// playlist would be a track the listener paid for and cannot see.
fn summary_is(summary: &TrackSummary, title: &str, artist: &str) -> bool {
    summary.title.eq_ignore_ascii_case(title)
        && summary
            .artist
            .as_deref()
            .is_some_and(|known| known.eq_ignore_ascii_case(artist))
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

/// One waiting decision, as a screen needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewCard {
    /// Which decision this is.
    pub id: ImportReviewId,
    /// Why the file is waiting.
    pub reason: ReviewReason,
    /// Where the file is. Absent only if the catalogue row went with it.
    pub path: Option<PathBuf>,
    /// The track it duplicates, when that is what it is.
    pub existing: Option<TrackSummary>,
}

/// A field the listener left blank, as the absence it is.
///
/// An empty artist means "no artist", not "an artist called nothing": the
/// column already carries that distinction and somebody clearing a field is
/// using it.
fn blank_as_absent(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// Which review entry a failed import deserves, if it deserves one at all.
///
/// **Listed one by one, so that a new `CoreError` variant stops the build
/// instead of quietly becoming "the file vanished".**
///
/// Two variants used to be named and a wildcard took the other nine -
/// `Storage`, `Cancelled` and `NoActiveProfile` among them - into
/// `MissingFile`, whose own documentation says the file disappeared between
/// being seen and being imported. That reason is written to
/// `import_review.reason` and shown to the listener, so a database busy for a
/// moment sent somebody looking for a file they had not lost.
///
/// Only three of the eleven are about the file in front of us. `None` is the
/// rest: they are about the run rather than the file. A review entry is the
/// wrong place for those, because it asks the listener to decide something they
/// cannot affect, and it outlives the failure - the next scan finds the file
/// perfectly readable.
fn review_reason(err: &CoreError) -> Option<ReviewReason> {
    match err {
        CoreError::Metadata(_) => Some(ReviewReason::UnreadableMetadata),
        CoreError::Decode(_) => Some(ReviewReason::UndecodableAudio),
        CoreError::FileSystem(_) => Some(ReviewReason::MissingFile),

        CoreError::Invalid { .. }
        | CoreError::NotFound { .. }
        | CoreError::Conflict(_)
        | CoreError::NoActiveProfile
        | CoreError::Storage(_)
        | CoreError::Audio(_)
        | CoreError::Analysis(_)
        | CoreError::Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::review_reason;
    use crate::domain::review::ReviewReason;
    use crate::error::CoreError;

    #[test]
    fn only_the_file_s_own_faults_reach_the_listener() {
        assert_eq!(
            review_reason(&CoreError::Decode("no frames".into())),
            Some(ReviewReason::UndecodableAudio)
        );
        assert_eq!(
            review_reason(&CoreError::FileSystem("gone".into())),
            Some(ReviewReason::MissingFile)
        );

        // The one that sent somebody looking for a file they still had.
        assert_eq!(review_reason(&CoreError::Storage("busy".into())), None);
        assert_eq!(review_reason(&CoreError::Cancelled), None);
        assert_eq!(review_reason(&CoreError::NoActiveProfile), None);
    }
}
