//! A throwaway database on disk, migrated and ready to use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use cadenza_infra::db::{self, SqlitePool};

/// Distinguishes databases created within one test binary.
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
        // The process id alone is not unique: Windows reissues them, and a
        // removal that failed on the last run leaves a database behind for the
        // next process to be given that number — which then opens somebody
        // else's data and fails on a unique constraint, a very long way from
        // anything the test is about. The start time makes the name unique
        // whatever the operating system does with process ids, and the counter
        // separates the tests running side by side within one binary.
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());

        let directory = env::temp_dir().join(format!(
            "cadenza-test-{}-{stamp:x}-{}",
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

        // Best effort, twice. Windows can hold a just-closed file open for a
        // moment, and one failed removal used to mean a directory left in the
        // system temp folder for good — thousands of them, over a project.
        //
        // Still only best effort: a panic during unwinding aborts the process
        // and hides whichever assertion actually failed, which is far worse
        // than litter.
        if fs::remove_dir_all(&self.directory).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
