//! Storing cover images outside the database.

use std::path::PathBuf;

use crate::Result;
use crate::domain::ids::MediaFileId;

/// Cover art storage.
///
/// Artwork lives on disk under `%LOCALAPPDATA%` rather than in SQLite
/// (PROJECT_MASTER 6): images are large, rarely queried, and entirely
/// regenerable from the source files, so keeping them out of the database keeps
/// it small and its backups cheap.
///
/// Cached per media file, not per profile: the image belongs to the recording.
pub trait ArtworkCachePort: Send + Sync {
    /// Stores an image, replacing any previous one for this file.
    fn store(&self, media_file_id: MediaFileId, image: &[u8]) -> Result<()>;

    /// Where the cached image lives, if it has been cached.
    fn path_for(&self, media_file_id: MediaFileId) -> Option<PathBuf>;

    /// Drops the cached image. Safe to call when nothing is cached.
    fn remove(&self, media_file_id: MediaFileId) -> Result<()>;
}
