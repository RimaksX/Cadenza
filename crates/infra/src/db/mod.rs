//! SQLite storage.
//!
//! Three places where the schema deliberately departs from the one first
//! written down:
//!
//! * **Timestamps are `INTEGER` unix milliseconds, never `TEXT`.** They match
//!   [`cadenza_core::domain::value_objects::Timestamp`], sort and index
//!   correctly, and need no calendar library. **`daily_*.date` stays `TEXT` on
//!   purpose** — a civil date is a different thing from an instant.
//! * **No JSON blobs for settings or metadata overrides.** Typed columns
//!   instead: two writable copies of one fact drift apart, and a blob cannot be
//!   queried, indexed or constrained.
//! * **No `track_features.scale`** — it and `mode` name the same major/minor
//!   property.
//!
//! `CHECK` constraints repeat what the domain already forbids, so the database
//! rejects a bad state even if something reaches it without going through core.

use std::fs;
use std::path::Path;

use cadenza_core::{CoreError, Result};

pub mod backup;
pub mod error;
pub mod migrations;
pub mod pool;
pub mod repositories;
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
