//! Profile storage.

use cadenza_core::domain::ids::ProfileId;
use cadenza_core::domain::ports::repositories::ProfileRepositoryPort;
use cadenza_core::domain::profile::{Profile, ProfileName};
use cadenza_core::domain::value_objects::{ThemeMode, Timestamp};
use cadenza_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Row};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, name, created_at, history_enabled, history_retention_days, theme";

/// Reads and writes `profiles`.
pub struct SqliteProfileRepository {
    pool: SqlitePool,
}

impl SqliteProfileRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl ProfileRepositoryPort for SqliteProfileRepository {
    fn list(&self) -> Result<Vec<Profile>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            // Case-insensitive so "kim" does not sort after "Sasha", which is
            // what a listener reading an alphabetical list expects.
            .prepare(&format!(
                "SELECT {COLUMNS} FROM profiles ORDER BY name COLLATE NOCASE"
            ))
            .map_err(db_error_in("listing profiles"))?;

        let rows = statement
            .query_map([], ProfileRow::read)
            .map_err(db_error_in("listing profiles"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing profiles"))?;

        rows.into_iter().map(ProfileRow::into_domain).collect()
    }

    fn get(&self, id: ProfileId) -> Result<Option<Profile>> {
        let connection = self.pool.get()?;
        let row = connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM profiles WHERE id = ?1"),
                [id.to_string()],
                ProfileRow::read,
            )
            .optional()
            .map_err(db_error_in("reading a profile"))?;

        row.map(ProfileRow::into_domain).transpose()
    }

    fn save(&self, profile: &Profile) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                // `created_at` is deliberately absent from the update: when a
                // profile was created does not change when it is renamed.
                "INSERT INTO profiles
                     (id, name, created_at, history_enabled, history_retention_days, theme)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     name                   = excluded.name,
                     history_enabled        = excluded.history_enabled,
                     history_retention_days = excluded.history_retention_days,
                     theme                  = excluded.theme",
                (
                    profile.id.to_string(),
                    profile.name.as_str(),
                    profile.created_at.as_millis(),
                    profile.history_enabled,
                    // Written through the domain's cap, so a profile can never
                    // be persisted with a wider retention window than the policy
                    // allows even if the field was set directly.
                    profile.effective_retention_days(),
                    profile.theme.as_str(),
                ),
            )
            .map_err(db_error_in("saving a profile"))?;
        Ok(())
    }

    fn delete(&self, id: ProfileId) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            // Everything scoped to the profile goes with it through ON DELETE
            // CASCADE. The shared catalogue and the files on disk are untouched.
            .execute("DELETE FROM profiles WHERE id = ?1", [id.to_string()])
            .map_err(db_error_in("deleting a profile"))?;
        Ok(())
    }
}

/// Column values as stored, before domain validation.
struct ProfileRow {
    id: String,
    name: String,
    created_at: i64,
    history_enabled: bool,
    history_retention_days: i64,
    theme: String,
}

impl ProfileRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            created_at: row.get("created_at")?,
            history_enabled: row.get("history_enabled")?,
            history_retention_days: row.get("history_retention_days")?,
            theme: row.get("theme")?,
        })
    }

    /// Validates the row into an entity.
    ///
    /// The schema already constrains all of this, so a failure here means the
    /// file was edited by hand or written by a different program. Reporting it
    /// beats constructing a profile the domain considers impossible.
    fn into_domain(self) -> Result<Profile> {
        let retention = u16::try_from(self.history_retention_days).map_err(|_| {
            CoreError::invalid(
                "history retention",
                format!(
                    "{} is not a plausible number of days",
                    self.history_retention_days
                ),
            )
        })?;

        Ok(Profile {
            id: ProfileId::parse(&self.id)?,
            name: ProfileName::new(self.name)?,
            created_at: Timestamp::from_millis(self.created_at),
            history_enabled: self.history_enabled,
            history_retention_days: retention,
            theme: ThemeMode::parse(&self.theme)?,
        })
    }
}
