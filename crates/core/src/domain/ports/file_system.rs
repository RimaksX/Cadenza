//! Reading the filesystem.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::domain::value_objects::Timestamp;

/// What the scanner needs to know about a path without opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetadata {
    /// Size in bytes.
    pub size: u64,
    /// Last modification time.
    pub modified: Timestamp,
    /// True for directories.
    pub is_dir: bool,
}

/// Filesystem access.
///
/// Read-only by design. Cadenza never deletes a file from disk: removing a
/// track removes it from the library only, and tag writing is not required, so
/// no write method exists to be called by mistake.
pub trait FileSystemPort: Send + Sync {
    /// True when the path resolves.
    fn exists(&self, path: &Path) -> bool;

    /// Size, modification time and kind.
    fn metadata(&self, path: &Path) -> Result<FileMetadata>;

    /// Immediate children of a directory, files and subdirectories alike.
    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;

    /// A content hash for duplicate detection.
    ///
    /// Reads the whole file, so it is a background job rather than something
    /// the scanner does inline.
    fn hash_file(&self, path: &Path) -> Result<String>;

    /// Creates a directory and every parent it needs.
    ///
    /// Creating one that is already there is not an error: the caller wants it
    /// to exist, not to have been the one who made it.
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Reads a whole file.
    ///
    /// For the small ones only — a chosen cover, and nothing else so far. Audio
    /// is streamed by the decoder and hashed by [`Self::hash_file`], neither of
    /// which wants a copy of a hundred megabytes in memory.
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
}
