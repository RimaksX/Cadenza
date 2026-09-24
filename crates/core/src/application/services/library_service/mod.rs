//! Scanning folders and importing what is found.
//!
//! The decisions live here — is this file new, changed, a duplicate, or a
//! problem — while the mechanisms (walking directories, reading tags, hashing)
//! sit behind ports in the infrastructure layer.

mod collections;
mod covers;
mod fetch;
mod folders;
mod import;
mod playlist_files;
mod reviews;
mod tracks;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use collections::ArtistSummary;
pub use playlist_files::PlaylistImport;

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
use crate::domain::ports::saved_file::SavedFilePort;
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
    /// Where a playlist written out for another player goes: the one file
    /// this service ever writes, and only to a place somebody chose.
    pub saved: Arc<dyn SavedFilePort>,
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
/// **Listed one by one, so a new `CoreError` variant stops the build instead of
/// quietly becoming "the file vanished".** A wildcard here once sent `Storage`
/// and `Cancelled` to `MissingFile`, and that reason reaches the listener — a
/// database busy for a moment sent somebody looking for a file they still had.
///
/// Only three of the eleven are about the file itself. `None` is the rest: a
/// review asks the listener to decide something, and they cannot decide a busy
/// database.
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
