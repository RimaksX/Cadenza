//! Writing a file the listener asked for.

use std::path::Path;

use crate::Result;

/// Writes one file, to a place the listener chose in a save dialog.
///
/// Apart from [`FileSystemPort`](super::file_system::FileSystemPort), which is
/// read-only so that nothing can write to the music by mistake. This one is
/// handed a path only after somebody named it, and it is the only way
/// anything leaves this application as a file.
pub trait SavedFilePort: Send + Sync {
    /// Writes `contents` to `path`, replacing what is there: the dialog that
    /// chose the path has already asked about that.
    fn write(&self, path: &Path, contents: &[u8]) -> Result<()>;
}
