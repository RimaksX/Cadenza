//! SQLite storage.
//!
//! # Where the schema differs from PROJECT_MASTER section 7
//!
//! Three deliberate corrections, all recorded in `docs/MASTER_ISSUES.md`:
//!
//! * **Timestamps are `INTEGER` unix milliseconds, not `TEXT`.** Section 7 is
//!   already inconsistent about this — `media_files.file_mtime` is `INTEGER`
//!   while `media_files.created_at` is `TEXT`, and both are instants. Integers
//!   match the domain's [`cadenza_core::domain::value_objects::Timestamp`]
//!   exactly, sort and index correctly, and need no calendar library to read or
//!   write. `daily_*.date` stays `TEXT`: a civil date is genuinely a different
//!   thing from an instant.
//! * **`profiles.settings_json` is dropped** in favour of the `profile_settings`
//!   table, and **`profile_tracks.metadata_override_json` is dropped** in favour
//!   of the typed override columns beside it. Two writable copies of one fact
//!   drift apart, and a JSON blob cannot be queried, indexed or constrained.
//! * **`track_features.scale` is dropped.** It and `mode` name the same
//!   major/minor property.
//!
//! Everything else follows section 7 as written, with `CHECK` constraints added
//! so the database rejects the states the domain already forbids.

use std::fs;
use std::path::Path;

use cadenza_core::{CoreError, Result};

pub mod error;
pub mod migrations;
pub mod pool;
pub mod sqlite;

pub use error::db_error;
pub use pool::{PooledConnection, SqlitePool};

/// Opens the database, creating and migrating it if necessary.
///
/// This is the one entry point the composition root should use. Migrations run
/// on a dedicated connection before the pool is filled, so no pooled connection
/// can ever observe a half-migrated schema.
pub fn open(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            CoreError::Storage(format!(
                "could not create the database directory {}: {err}",
                parent.display()
            ))
        })?;
    }

    let mut connection = sqlite::open(path)?;
    migrations::apply(&mut connection)?;
    drop(connection);

    SqlitePool::open(path)
}
