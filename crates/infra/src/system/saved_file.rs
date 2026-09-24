//! Writing a file the listener chose a place for.

use std::fs;
use std::path::Path;

use cadenza_core::domain::ports::saved_file::SavedFilePort;
use cadenza_core::{CoreError, Result};

/// Writes straight to the local disk.
pub struct LocalSavedFiles;

impl SavedFilePort for LocalSavedFiles {
    fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        fs::write(path, contents).map_err(|err| {
            CoreError::FileSystem(format!("could not write {}: {err}", path.display()))
        })
    }
}
