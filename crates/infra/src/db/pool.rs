//! A small fixed-size pool of configured connections.
//!
//! # Why a pool at all
//!
//! `rusqlite::Connection` is `Send` but not `Sync`, so it cannot simply be shared.
//! The obvious alternative — one connection behind a `Mutex` — would serialise
//! everything, and the whole point of WAL is that a background scan can write
//! while the UI reads. A handful of connections buys that concurrency for about
//! sixty lines.

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cadenza_core::{CoreError, Result};
use rusqlite::Connection;

use super::sqlite;

/// Connections opened by default.
///
/// A calibration knob. One for the UI, one for the audio/queue writer, one for
/// the background scanner and one spare covers the threads Cadenza actually
/// runs. More would not help: WAL still admits a single writer at a time.
pub const DEFAULT_POOL_SIZE: usize = 4;

/// How long a caller waits for a free connection before failing.
///
/// Waiting forever would turn a leaked connection or a self-deadlock into a
/// hung application with no diagnostic. Thirty seconds is long enough that no
/// healthy checkout hits it, and short enough that a bug reports itself.
pub const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(30);

struct Shared {
    idle: Mutex<Vec<Connection>>,
    available: Condvar,
}

/// A fixed-size pool of connections to one database file.
///
/// Cloning is cheap and shares the same connections.
#[derive(Clone)]
pub struct SqlitePool {
    shared: Arc<Shared>,
    path: PathBuf,
    size: usize,
}

impl SqlitePool {
    /// Opens a pool of [`DEFAULT_POOL_SIZE`] connections.
    pub fn open(path: &Path) -> Result<Self> {
        Self::with_size(path, DEFAULT_POOL_SIZE)
    }

    /// Opens a pool of exactly `size` connections.
    ///
    /// Every connection is opened up front. Lazy creation would only move the
    /// failure — a bad path, a corrupt file — from startup to some later moment
    /// in the middle of the user's work.
    pub fn with_size(path: &Path, size: usize) -> Result<Self> {
        if size == 0 {
            return Err(CoreError::invalid("pool size", "must be at least 1"));
        }

        let mut connections = Vec::with_capacity(size);
        for _ in 0..size {
            connections.push(sqlite::open(path)?);
        }

        Ok(Self {
            shared: Arc::new(Shared {
                idle: Mutex::new(connections),
                available: Condvar::new(),
            }),
            path: path.to_path_buf(),
            size,
        })
    }

    /// Takes a connection, waiting up to [`CHECKOUT_TIMEOUT`] for a free one.
    ///
    /// The connection returns to the pool when the guard is dropped, including
    /// on an early return or a panic. Do not hold two at once from the same
    /// thread: with a small pool that is how you deadlock yourself.
    pub fn get(&self) -> Result<PooledConnection> {
        let mut idle = self.shared.idle.lock().map_err(|_| poisoned())?;

        loop {
            if let Some(connection) = idle.pop() {
                return Ok(PooledConnection {
                    shared: Arc::clone(&self.shared),
                    connection: Some(connection),
                });
            }

            let (guard, wait) = self
                .shared
                .available
                .wait_timeout(idle, CHECKOUT_TIMEOUT)
                .map_err(|_| poisoned())?;
            idle = guard;

            if wait.timed_out() && idle.is_empty() {
                return Err(CoreError::Storage(format!(
                    "no database connection became free within {} seconds; \
                     all {} are still checked out",
                    CHECKOUT_TIMEOUT.as_secs(),
                    self.size
                )));
            }
        }
    }

    /// How many connections the pool holds in total.
    pub const fn size(&self) -> usize {
        self.size
    }

    /// The database file this pool is connected to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlitePool")
            .field("path", &self.path)
            .field("size", &self.size)
            .finish()
    }
}

/// A connection borrowed from a [`SqlitePool`], returned on drop.
pub struct PooledConnection {
    shared: Arc<Shared>,
    /// Always `Some` until [`Drop`] takes it back.
    connection: Option<Connection>,
}

impl Deref for PooledConnection {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.connection
            .as_ref()
            .expect("the connection is only taken in Drop")
    }
}

impl DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection
            .as_mut()
            .expect("the connection is only taken in Drop")
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };

        // A poisoned pool means another thread panicked holding the lock. Drop
        // the connection rather than propagating: panicking inside Drop during
        // unwinding aborts the process, and losing one connection from the pool
        // is a far smaller problem than that.
        if let Ok(mut idle) = self.shared.idle.lock() {
            idle.push(connection);
            self.shared.available.notify_one();
        }
    }
}

fn poisoned() -> CoreError {
    CoreError::Storage("the database connection pool was poisoned by a panic".to_owned())
}
