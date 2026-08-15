//! Schema migrations.
//!
//! Migrations are append-only. A migration that has shipped is never edited —
//! any change is a new numbered file — because an existing database will not
//! re-run it, so editing one guarantees two users on the same version have
//! different schemas.
//!
//! Each migration runs inside a transaction together with the row recording it.
//! A failure part-way leaves the database exactly as it was.

use cadenza_core::{CoreError, Result};
use rusqlite::{Connection, OptionalExtension};

use super::error::db_error_in;

mod m0001_initial;
mod m0002_profiles;
mod m0003_media_catalog;
mod m0004_library_entities;
mod m0005_profile_library;
mod m0006_playlists;
mod m0007_play_history;
mod m0008_radio;
mod m0009_eq;
mod m0010_settings;
mod m0011_analysis;
mod m0012_review;
mod m0013_profile_genres;
mod m0014_queue;
mod m0015_builtin_eq_presets;
mod m0016_preset_tone_controls;

/// One numbered schema change.
pub struct Migration {
    /// Sequence number, starting at 1.
    pub version: i64,
    /// Short name, recorded alongside the version for readability.
    pub name: &'static str,
    /// The statements to run.
    pub sql: &'static str,
}

/// Every migration, in the order they must be applied.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: m0001_initial::SQL,
    },
    Migration {
        version: 2,
        name: "profiles",
        sql: m0002_profiles::SQL,
    },
    Migration {
        version: 3,
        name: "media_catalog",
        sql: m0003_media_catalog::SQL,
    },
    Migration {
        version: 4,
        name: "library_entities",
        sql: m0004_library_entities::SQL,
    },
    Migration {
        version: 5,
        name: "profile_library",
        sql: m0005_profile_library::SQL,
    },
    Migration {
        version: 6,
        name: "playlists",
        sql: m0006_playlists::SQL,
    },
    Migration {
        version: 7,
        name: "play_history",
        sql: m0007_play_history::SQL,
    },
    Migration {
        version: 8,
        name: "radio",
        sql: m0008_radio::SQL,
    },
    Migration {
        version: 9,
        name: "eq",
        sql: m0009_eq::SQL,
    },
    Migration {
        version: 10,
        name: "settings",
        sql: m0010_settings::SQL,
    },
    Migration {
        version: 11,
        name: "analysis",
        sql: m0011_analysis::SQL,
    },
    Migration {
        version: 12,
        name: "review",
        sql: m0012_review::SQL,
    },
    Migration {
        version: 13,
        name: "profile_genres",
        sql: m0013_profile_genres::SQL,
    },
    Migration {
        version: 14,
        name: "queue",
        sql: m0014_queue::SQL,
    },
    Migration {
        version: 15,
        name: "builtin_eq_presets",
        sql: m0015_builtin_eq_presets::SQL,
    },
    Migration {
        version: 16,
        name: "preset_tone_controls",
        sql: m0016_preset_tone_controls::SQL,
    },
];

/// The schema version this build produces.
pub const LATEST_VERSION: i64 = MIGRATIONS[MIGRATIONS.len() - 1].version;

// Versions must be contiguous and ascending. Adding a migration out of order, or
// reusing a number, fails the build rather than corrupting somebody's database.
const _: () = {
    let mut index = 0;
    while index < MIGRATIONS.len() {
        assert!(
            MIGRATIONS[index].version == (index as i64) + 1,
            "migration versions must start at 1 and increase by one"
        );
        index += 1;
    }
};

/// Applies every outstanding migration, returning the versions that ran.
///
/// Idempotent: calling it on an up-to-date database does nothing and returns an
/// empty list.
pub fn apply(connection: &mut Connection) -> Result<Vec<i64>> {
    let applied = applied_versions(connection)?;

    // A database written by a newer build has tables and columns this code does
    // not know about. Reading it would be guesswork and writing to it could lose
    // data, so refuse rather than improvise.
    if let Some(&newest) = applied.last()
        && newest > LATEST_VERSION
    {
        return Err(CoreError::Storage(format!(
            "the database is at schema version {newest} but this build only knows \
             version {LATEST_VERSION}; it was created by a newer version of Cadenza"
        )));
    }

    let mut just_applied = Vec::new();
    for migration in MIGRATIONS {
        if applied.binary_search(&migration.version).is_ok() {
            continue;
        }

        let context = format!(
            "applying migration {:04} ({})",
            migration.version, migration.name
        );

        let transaction = connection.transaction().map_err(db_error_in(&context))?;
        transaction
            .execute_batch(migration.sql)
            .map_err(db_error_in(&context))?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at)
                 VALUES (?1, ?2, CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
                (migration.version, migration.name),
            )
            .map_err(db_error_in(&context))?;
        transaction.commit().map_err(db_error_in(&context))?;

        just_applied.push(migration.version);
    }

    Ok(just_applied)
}

/// Versions already recorded in the database, ascending.
///
/// Returns an empty list for a database that has never been migrated, which is
/// how a fresh file bootstraps: `schema_migrations` is created by migration 1.
pub fn applied_versions(connection: &Connection) -> Result<Vec<i64>> {
    let bookkeeping_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(db_error_in("looking for the migration table"))?
        .is_some();

    if !bookkeeping_exists {
        return Ok(Vec::new());
    }

    ensure_bookkeeping_shape(connection)?;

    let mut statement = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .map_err(db_error_in("reading applied migrations"))?;
    let versions = statement
        .query_map([], |row| row.get(0))
        .map_err(db_error_in("reading applied migrations"))?
        .collect::<rusqlite::Result<Vec<i64>>>()
        .map_err(db_error_in("reading applied migrations"))?;

    Ok(versions)
}

/// The schema version currently in the database, or `None` if it has none.
pub fn current_version(connection: &Connection) -> Result<Option<i64>> {
    Ok(applied_versions(connection)?.last().copied())
}

/// Columns migration 1 gives `schema_migrations`.
const BOOKKEEPING_COLUMNS: [&str; 3] = ["version", "name", "applied_at"];

/// Refuses a `schema_migrations` table that is not the one migration 1 creates.
///
/// A file can carry a table of that name written by something else — a different
/// program, or a pre-release build of Cadenza whose schema no longer matches.
/// Without this check the version numbers in it would be read as Cadenza's own,
/// the migrations they name would be skipped, and the first one that did run
/// would fail with a message about a missing column five migrations away from
/// the actual problem.
///
/// Refusing is the only safe answer: the rows are somebody's data, and there is
/// no way to tell what they mean.
fn ensure_bookkeeping_shape(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info('schema_migrations')")
        .map_err(db_error_in("inspecting the migration table"))?;

    let columns = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_error_in("inspecting the migration table"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error_in("inspecting the migration table"))?;

    let missing: Vec<&str> = BOOKKEEPING_COLUMNS
        .into_iter()
        .filter(|expected| !columns.iter().any(|column| column == expected))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    Err(CoreError::Storage(format!(
        "this file has a schema_migrations table without {missing:?}, so it was not \
         written by this version of Cadenza (it has {columns:?}). Refusing to migrate it: \
         move the file aside and let Cadenza create a new one."
    )))
}
