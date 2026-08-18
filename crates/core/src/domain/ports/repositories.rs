//! Storage contracts.
//!
//! One trait per aggregate rather than one god-repository, so a service declares
//! exactly what it touches and a test fake only implements what it needs.
//!
//! Methods are added when a milestone needs them. Guessing at a full query
//! surface now would produce a large trait whose unused half still has to be
//! implemented by every fake.
//!
//! # Profile isolation
//!
//! Anything holding user data takes a [`ProfileId`] on every read. That is not
//! defensive style, it is the isolation rule of PROJECT_MASTER 12.1 made
//! impossible to forget: there is no "list all playlists" to call by accident.
//! The exceptions are the global catalogue — media files, artists, albums,
//! genres, features, analysis jobs — which describe the music rather than the
//! listener.

use std::path::Path;

use crate::Result;
use crate::domain::album::Album;
use crate::domain::analysis::{AnalysisJob, AnalysisKind};
use crate::domain::artist::Artist;
use crate::domain::eq::EqPreset;
use crate::domain::genre::Genre;
use crate::domain::ids::{
    AlbumId, AnalysisJobId, ArtistId, EqPresetId, GenreId, ImportReviewId, MediaFileId, MoodId,
    PlaylistId, PlaylistItemId, ProfileId, RadioSessionId,
};
use crate::domain::media_file::{FileState, MediaFile};
use crate::domain::mood::MoodPreset;
use crate::domain::playlist::{Playlist, PlaylistItem};
use crate::domain::profile::Profile;
use crate::domain::queue::Queue;
use crate::domain::radio::{RadioFeedback, RadioSession, RadioSessionItem};
use crate::domain::review::{ImportReview, ReviewState};
use crate::domain::settings::{ProfileFolder, SettingValue};
use crate::domain::stats::{ListeningSummary, PlayEvent};
use crate::domain::track::{Track, TrackFeatures, TrackSummary};
use crate::domain::value_objects::Timestamp;

/// Listener profiles.
pub trait ProfileRepositoryPort: Send + Sync {
    /// Every profile, ordered by name.
    fn list(&self) -> Result<Vec<Profile>>;

    /// One profile.
    fn get(&self, id: ProfileId) -> Result<Option<Profile>>;

    /// Inserts or updates.
    fn save(&self, profile: &Profile) -> Result<()>;

    /// Deletes a profile and everything scoped to it.
    fn delete(&self, id: ProfileId) -> Result<()>;
}

/// Application and per-profile settings, plus library folders.
///
/// Values cross this boundary as [`SettingValue`], not as JSON text. The domain
/// has no business assembling or parsing an encoding; the adapter owns that, and
/// replacing JSON with something else would not touch a line of `core`.
pub trait SettingsRepositoryPort: Send + Sync {
    /// A global setting, such as which profile is active.
    fn app_get(&self, key: &str) -> Result<Option<SettingValue>>;

    /// Writes a global setting.
    fn app_set(&self, key: &str, value: &SettingValue, now: Timestamp) -> Result<()>;

    /// Forgets a global setting. Removing one that was never set is not an error.
    fn app_remove(&self, key: &str) -> Result<()>;

    /// A profile-scoped setting.
    fn profile_get(&self, profile_id: ProfileId, key: &str) -> Result<Option<SettingValue>>;

    /// Writes a profile-scoped setting.
    fn profile_set(
        &self,
        profile_id: ProfileId,
        key: &str,
        value: &SettingValue,
        now: Timestamp,
    ) -> Result<()>;

    /// Forgets a profile-scoped setting.
    fn profile_remove(&self, profile_id: ProfileId, key: &str) -> Result<()>;

    /// Library folders belonging to a profile.
    fn list_folders(&self, profile_id: ProfileId) -> Result<Vec<ProfileFolder>>;

    /// Inserts or updates a library folder.
    fn save_folder(&self, folder: &ProfileFolder) -> Result<()>;

    /// Removes a library folder. Files already imported are unaffected.
    fn delete_folder(&self, folder: &ProfileFolder) -> Result<()>;
}

/// The global catalogue of physical files.
pub trait MediaFileRepositoryPort: Send + Sync {
    /// One file by identifier.
    fn get(&self, id: MediaFileId) -> Result<Option<MediaFile>>;

    /// One file by path, which is unique across the catalogue.
    fn find_by_path(&self, path: &Path) -> Result<Option<MediaFile>>;

    /// Every file sharing a content hash — the duplicate-detection query.
    fn find_by_hash(&self, hash: &str) -> Result<Vec<MediaFile>>;

    /// Inserts or updates.
    fn save(&self, media_file: &MediaFile) -> Result<()>;

    /// Marks a file present, missing or unreadable.
    fn set_state(&self, id: MediaFileId, state: FileState, now: Timestamp) -> Result<()>;

    /// Moves a catalogue row to a new path.
    ///
    /// A renamed file is the same recording. Deleting the old row and inserting
    /// a new one would give it a new identity and take its listening history and
    /// playlist membership with it.
    fn set_path(&self, id: MediaFileId, path: &Path, now: Timestamp) -> Result<()>;
}

/// Per-profile library membership.
pub trait TrackRepositoryPort: Send + Sync {
    /// One track as a profile sees it.
    fn get(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<Option<Track>>;

    /// Every track currently in a profile's library, excluding tombstones.
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<Track>>;

    /// The same library, with artist and album names and durations resolved.
    ///
    /// Separate from [`Self::list_for_profile`] because a listing needs names
    /// and an edit needs identifiers, and resolving the names row by row would
    /// turn one query into thousands.
    fn summaries_for_profile(&self, profile_id: ProfileId) -> Result<Vec<TrackSummary>>;

    /// One row of a listing.
    fn summary(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Result<Option<TrackSummary>>;

    /// Inserts or updates.
    fn save(&self, track: &Track) -> Result<()>;

    /// Tombstones a track. The file stays on disk and in the catalogue.
    fn remove(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        now: Timestamp,
    ) -> Result<()>;

    /// Restores a tombstoned track, for when a listener re-adds a file they
    /// removed earlier. Their title and artist edits come back with it.
    fn restore(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<()>;
}

/// The global artist catalogue.
pub trait ArtistRepositoryPort: Send + Sync {
    /// One artist.
    fn get(&self, id: ArtistId) -> Result<Option<Artist>>;

    /// Finds an artist by exact name, for deduplicating during import.
    fn find_by_name(&self, name: &str) -> Result<Option<Artist>>;

    /// Inserts or updates.
    fn save(&self, artist: &Artist) -> Result<()>;
}

/// The global album catalogue.
pub trait AlbumRepositoryPort: Send + Sync {
    /// One album.
    fn get(&self, id: AlbumId) -> Result<Option<Album>>;

    /// Finds an album by title and album artist.
    fn find(&self, title: &str, artist_id: Option<ArtistId>) -> Result<Option<Album>>;

    /// Inserts or updates.
    fn save(&self, album: &Album) -> Result<()>;
}

/// The global genre vocabulary and its links to files.
pub trait GenreRepositoryPort: Send + Sync {
    /// Finds a genre by its normalised name.
    fn find_by_name(&self, name: &str) -> Result<Option<Genre>>;

    /// Inserts or updates.
    fn save(&self, genre: &Genre) -> Result<()>;

    /// Genres attached to a file, as its tags describe it.
    fn for_media_file(&self, media_file_id: MediaFileId) -> Result<Vec<Genre>>;

    /// Replaces the genres attached to a file. Seeded from tags by the scanner.
    fn set_for_media_file(&self, media_file_id: MediaFileId, genres: &[GenreId]) -> Result<()>;

    /// Genres as one profile sees them.
    ///
    /// Its own if it has corrected them, and the file's own otherwise. A profile
    /// that has deliberately cleared every genre sees none, which is a different
    /// answer from having never touched them (PROJECT_MASTER 2.1, 12.1).
    fn for_profile_track(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Result<Vec<Genre>>;

    /// Replaces what one profile sees, leaving the file and every other profile
    /// alone.
    fn set_for_profile_track(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        genres: &[GenreId],
    ) -> Result<()>;

    /// Drops the correction, so the file's own genres show again.
    fn clear_for_profile_track(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Result<()>;
}

/// Playlists and their contents.
pub trait PlaylistRepositoryPort: Send + Sync {
    /// Every playlist a profile owns.
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<Playlist>>;

    /// One playlist.
    fn get(&self, id: PlaylistId) -> Result<Option<Playlist>>;

    /// Inserts or updates.
    fn save(&self, playlist: &Playlist) -> Result<()>;

    /// Deletes a playlist and its entries. The tracks stay in the library.
    fn delete(&self, id: PlaylistId) -> Result<()>;

    /// Entries in playback order.
    fn items(&self, playlist_id: PlaylistId) -> Result<Vec<PlaylistItem>>;

    /// Replaces the entries wholesale.
    ///
    /// One call rather than per-item edits, because reordering by drag-and-drop
    /// renumbers many rows at once and doing that as separate writes would leave
    /// the positions briefly inconsistent.
    fn replace_items(&self, playlist_id: PlaylistId, items: &[PlaylistItem]) -> Result<()>;

    /// Removes one entry.
    fn delete_item(&self, id: PlaylistItemId) -> Result<()>;
}

/// The saved playback queue.
///
/// PROJECT_MASTER 2.3 requires restoring the last queue and 2.5 makes the queue
/// per-profile, but section 7 defines no table for it. The storage shape is
/// settled in M7 alongside the queue service; this port is the contract that
/// migration has to satisfy.
pub trait QueueRepositoryPort: Send + Sync {
    /// The queue as it was left, or `None` if the profile has never played.
    fn load(&self, profile_id: ProfileId) -> Result<Option<Queue>>;

    /// Persists the queue so it survives a restart or a profile switch.
    fn save(&self, queue: &Queue) -> Result<()>;

    /// Discards a profile's saved queue.
    fn clear(&self, profile_id: ProfileId) -> Result<()>;
}

/// Raw listening events.
pub trait PlayEventRepositoryPort: Send + Sync {
    /// Records one listen.
    ///
    /// Callers check [`crate::domain::policies::history_policy::should_record`]
    /// first; this port does not second-guess them.
    fn append(&self, event: &PlayEvent) -> Result<()>;

    /// Events for a profile since a point in time, most recent first.
    fn recent(&self, profile_id: ProfileId, since: Timestamp, limit: u32)
    -> Result<Vec<PlayEvent>>;

    /// Deletes everything older than the cutoff. Returns how many rows went.
    fn purge_before(&self, profile_id: ProfileId, cutoff: Timestamp) -> Result<u64>;

    /// Deletes every event for a profile, for when history is switched off.
    fn purge_all(&self, profile_id: ProfileId) -> Result<u64>;
}

/// Aggregated listening statistics.
///
/// The daily rollup tables of PROJECT_MASTER 7.4 and the dashboard queries that
/// read them arrive in M14. Only the query radio needs before then is declared
/// here.
pub trait StatsRepositoryPort: Send + Sync {
    /// Everything the dashboard says in one row.
    fn summary(&self, profile_id: ProfileId, since: Timestamp) -> Result<ListeningSummary>;

    /// Most-played files in a window, as `(file, play count)`, highest first.
    fn top_tracks(
        &self,
        profile_id: ProfileId,
        since: Timestamp,
        limit: u32,
    ) -> Result<Vec<(MediaFileId, u32)>>;
}

/// Radio sessions and the picks they made.
pub trait RadioRepositoryPort: Send + Sync {
    /// One session.
    fn get_session(&self, id: RadioSessionId) -> Result<Option<RadioSession>>;

    /// Inserts or updates a session.
    fn save_session(&self, session: &RadioSession) -> Result<()>;

    /// Appends a chosen track.
    fn append_item(&self, item: &RadioSessionItem) -> Result<()>;

    /// The most recent picks, newest last — what diversity rules read to avoid
    /// repeating an artist or genre too soon.
    fn recent_items(&self, session_id: RadioSessionId, limit: u32)
    -> Result<Vec<RadioSessionItem>>;

    /// Records what the listener thought of a pick.
    ///
    /// The most recent pick of that file in that session, because a track
    /// offered twice was judged the second time.
    fn set_feedback(
        &self,
        session_id: RadioSessionId,
        media_file_id: MediaFileId,
        feedback: RadioFeedback,
        now: Timestamp,
    ) -> Result<()>;

    /// When each file was last offered by any of this profile's stations.
    ///
    /// What the freshness term of PROJECT_MASTER 10.4 is measured against.
    /// Listening history would be the better source and is not written yet —
    /// that is M14's — but "how long since radio last played you this" is the
    /// question freshness is actually asking of a station, and radio has kept
    /// the answer since its first session (MASTER_ISSUES 49).
    fn last_offered(&self, profile_id: ProfileId) -> Result<Vec<(MediaFileId, Timestamp)>>;

    /// Every verdict a profile has given, summed per file.
    ///
    /// Summed rather than listed: what generation needs is "how does this
    /// listener feel about this track", and three skips and a like is one
    /// answer rather than four. The sign is [`RadioFeedback::weight`]'s.
    fn preferences(&self, profile_id: ProfileId) -> Result<Vec<(MediaFileId, f32)>>;
}

/// Mood and activity presets.
pub trait MoodRepositoryPort: Send + Sync {
    /// Built-in moods plus the profile's own.
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<MoodPreset>>;

    /// One preset.
    fn get(&self, id: MoodId) -> Result<Option<MoodPreset>>;

    /// Inserts or updates. Built-ins are rejected.
    fn save(&self, preset: &MoodPreset) -> Result<()>;

    /// Deletes a custom preset. Built-ins are rejected.
    fn delete(&self, id: MoodId) -> Result<()>;
}

/// Equaliser presets.
pub trait EqPresetRepositoryPort: Send + Sync {
    /// Built-in presets plus the profile's own.
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<EqPreset>>;

    /// One preset.
    fn get(&self, id: EqPresetId) -> Result<Option<EqPreset>>;

    /// Inserts or updates. Built-ins are rejected.
    fn save(&self, preset: &EqPreset) -> Result<()>;

    /// Deletes a custom preset. Built-ins are rejected.
    fn delete(&self, id: EqPresetId) -> Result<()>;
}

/// The background analysis work queue.
pub trait AnalysisJobRepositoryPort: Send + Sync {
    /// Enqueues work, ignoring a request that is already queued for the same
    /// file and kind.
    fn enqueue(&self, job: &AnalysisJob) -> Result<()>;

    /// Takes the highest-priority queued job and marks it running.
    ///
    /// Claiming and marking are one call so that two workers cannot pick up the
    /// same job.
    fn claim_next(&self, kind: Option<AnalysisKind>, now: Timestamp)
    -> Result<Option<AnalysisJob>>;

    /// Marks a job finished.
    fn complete(&self, id: AnalysisJobId, now: Timestamp) -> Result<()>;

    /// Records a failure, incrementing the attempt count.
    fn fail(&self, id: AnalysisJobId, error: &str, now: Timestamp) -> Result<()>;

    /// How much work is outstanding, for the progress indicator.
    fn pending_count(&self) -> Result<u64>;

    /// Queues feature extraction for files that still need it, and says how
    /// many it queued.
    ///
    /// A file needs it when it has no features from `extractor_version` and has
    /// not already spent its attempts failing. Both halves matter: without the
    /// first the worker would redo the whole library on every start, and
    /// without the second a file that cannot be decoded would be picked up for
    /// ever (PROJECT_MASTER M11, "повторный анализ не происходит").
    ///
    /// One call rather than a listing the caller loops over, because "which
    /// files still need this" is a question about rows the database can answer
    /// without sending five thousand identifiers across the boundary to be
    /// filtered and sent back.
    fn enqueue_missing_features(
        &self,
        extractor_version: &str,
        limit: usize,
        now: Timestamp,
    ) -> Result<u64>;
}

/// What analysis learned about a file.
///
/// Global rather than per-profile: the numbers describe the recording, and two
/// listeners sharing a file share what it sounds like.
pub trait TrackFeaturesRepositoryPort: Send + Sync {
    /// One file's features, if they have been extracted.
    fn get(&self, media_file_id: MediaFileId) -> Result<Option<TrackFeatures>>;

    /// Inserts or replaces a file's features.
    fn save(&self, features: &TrackFeatures) -> Result<()>;

    /// How many files carry features from this extractor.
    fn count_for_extractor(&self, extractor_version: &str) -> Result<u64>;

    /// Every file's features.
    ///
    /// The whole table rather than a query per candidate: smart shuffle scores
    /// what it could play against what is playing, and that is a question about
    /// the library rather than about one track. A library of five thousand is a
    /// few hundred kilobytes read once per track change, which is cheaper than
    /// five thousand round trips and far cheaper than getting it wrong.
    fn list_all(&self) -> Result<Vec<TrackFeatures>>;
}

/// Files awaiting an import decision.
pub trait ImportReviewRepositoryPort: Send + Sync {
    /// Entries in a given state for a profile.
    fn list_for_profile(
        &self,
        profile_id: ProfileId,
        state: ReviewState,
    ) -> Result<Vec<ImportReview>>;

    /// Inserts or updates.
    fn save(&self, review: &ImportReview) -> Result<()>;

    /// Marks an entry resolved or dismissed.
    fn set_state(&self, id: ImportReviewId, state: ReviewState, now: Timestamp) -> Result<()>;
}
