//! Opening a connection and putting it into the state the rest of the code assumes.

use std::path::Path;
use std::time::Duration;

use cadenza_core::{CoreError, Result};
use rusqlite::Connection;

use super::error::{db_error, db_error_in};

/// How long a statement waits for a lock before giving up.
///
/// WAL allows one writer at a time. Without a timeout a background scan writing
/// while the user saves a playlist produces `SQLITE_BUSY` instead of a short
/// wait. Five seconds is far longer than any statement here should take, so
/// hitting it means something is genuinely wrong rather than merely contended.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens a connection to a database file and configures it.
///
/// Every connection must go through here. `foreign_keys` is a per-connection
/// setting, so a connection opened any other way would silently skip referential
/// integrity — which is exactly the kind of thing that is noticed months later.
pub fn open(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path).map_err(db_error_in(&format!(
        "opening the database at {}",
        path.display()
    )))?;

    let journal_mode = configure(&connection)?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(CoreError::Storage(format!(
            "the database at {} is in {journal_mode} mode, not WAL; \
             concurrent reads during background scanning would block",
            path.display()
        )));
    }

    Ok(connection)
}

/// Applies the pragmas every connection needs, returning the journal mode in force.
///
/// `journal_mode` is persistent — it is a property of the database file, so only
/// the first connection actually changes anything. The rest are per-connection
/// and must be set every time.
pub fn configure(connection: &Connection) -> Result<String> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(db_error_in("setting busy_timeout"))?;

    // Returns a row, so it cannot go in the batch below.
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(db_error_in("enabling WAL"))?;

    connection
        .execute_batch(
            // Referential integrity. Per-connection, and off by default.
            "PRAGMA foreign_keys = ON;
             -- Safe to lose the last few committed transactions on power loss
             -- but never to corrupt the file. The full fsync per commit that
             -- `FULL` costs is not worth it for a music library.
             PRAGMA synchronous = NORMAL;
             -- Sorting and temporary tables stay off disk.
             PRAGMA temp_store = MEMORY;",
        )
        .map_err(db_error_in("applying connection pragmas"))?;

    Ok(journal_mode)
}

/// Reads back an integer pragma. Used by tests to prove the settings took effect.
pub fn pragma_i64(connection: &Connection, name: &str) -> Result<i64> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(db_error)
}
