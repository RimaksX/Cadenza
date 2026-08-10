//! A throwaway directory, deleted on drop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, fs, process};

/// Distinguishes directories created within one test binary. Combined with the
/// process id it is unique across parallel runs too.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// An empty directory under the system temporary folder.
///
/// For tests that need real files but no database — decoding, hashing, watching.
/// Borrowing [`super::TempDb`] for its directory would run twelve migrations to
/// produce a path.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates the directory.
    ///
    /// Panics on failure: a fixture that cannot be created should stop the test
    /// immediately rather than turn every call site into a `Result`.
    #[must_use]
    pub fn new(tag: &str) -> Self {
        let path = env::temp_dir().join(format!(
            "cadenza-{tag}-{}-{}",
            process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)
            .unwrap_or_else(|err| panic!("could not create {}: {err}", path.display()));

        Self { path }
    }

    /// The directory itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort: a leftover directory in the temporary folder is untidy,
        // and panicking here would replace a real test failure with this one.
        let _ = fs::remove_dir_all(&self.path);
    }
}
