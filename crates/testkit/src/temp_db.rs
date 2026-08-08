//! A throwaway database on disk, migrated and ready to use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, fs, process};

use cadenza_infra::db::{self, SqlitePool};

/// Distinguishes databases created within one test binary. Combined with the
/// process id it is unique across parallel test runs too, which matters because
/// `cargo test` runs tests on many threads and may run two binaries at once.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A migrated SQLite database in a temporary directory, deleted on drop.
///
/// On disk rather than in memory on purpose: an in-memory database cannot use
/// WAL, so it would not exercise the journal mode, the file layout or the
/// concurrency the real application depends on. A test that passes against a
/// different configuration than production runs is not worth much.
pub struct TempDb {
    directory: PathBuf,
    /// `Some` until [`Drop`] closes the connections.
    pool: Option<SqlitePool>,
}

impl TempDb {
    /// Creates and migrates a fresh database.
    ///
    /// Panics on failure. This is a test helper: a broken fixture should stop the
    /// test immediately and loudly, not turn into a `Result` every test has to
    /// unwrap anyway.
    #[must_use]
    pub fn new() -> Self {
        let directory = env::temp_dir().join(format!(
            "cadenza-test-{}-{}",
            process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory)
            .unwrap_or_else(|err| panic!("could not create {}: {err}", directory.display()));

        let pool = db::open(&directory.join("app.db")).unwrap_or_else(|err| {
            panic!(
                "could not open the test database in {}: {err}",
                directory.display()
            )
        });

        Self {
            directory,
            pool: Some(pool),
        }
    }

    /// The pool of connections to this database.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        self.pool.as_ref().expect("the pool is only taken in Drop")
    }

    /// The database file.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.pool().path()
    }

    /// The directory holding the database and its WAL sidecars.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Default for TempDb {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        // Connections first: Windows refuses to delete a directory holding an
        // open file, so dropping the pool has to happen before the removal.
        drop(self.pool.take());

        // Best effort. A leftover directory in the system temp folder is a far
        // smaller problem than a panic during unwinding, which aborts the
        // process and hides whichever assertion actually failed.
        let _ = fs::remove_dir_all(&self.directory);
    }
}
