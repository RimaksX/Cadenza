//! A copy of everything Cadenza keeps, and putting one back.

use std::path::Path;

use crate::Result;

/// Saves and restores the whole of what this machine's listeners keep: the
/// database, and the pictures they chose, which nothing could make again.
/// The covers read out of the music files are left out: the files still have
/// them.
///
/// Restoring does not happen while the application runs. The copy is checked
/// and set aside, and it replaces what is there on the next start, before
/// anything has the database open; the data it replaces is kept beside it.
pub trait BackupPort: Send + Sync {
    /// Writes a copy to `to`.
    fn save(&self, to: &Path) -> Result<()>;

    /// Checks that `from` is a copy this version can read, and sets it to
    /// replace the current data on the next start.
    fn stage_restore(&self, from: &Path) -> Result<()>;
}
