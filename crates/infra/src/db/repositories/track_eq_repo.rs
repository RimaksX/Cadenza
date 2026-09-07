//! Which preset a listener has chosen for one track.
//!
//! Three statements and no reading of rows worth the name: what comes back is
//! one identifier, and the whole point of the table is that it holds a pointer
//! rather than a curve (`MASTER_ISSUES` 89).

use cadenza_core::domain::ids::{EqPresetId, MediaFileId, ProfileId};
use cadenza_core::domain::ports::repositories::TrackEqRepositoryPort;
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::Result;
use rusqlite::OptionalExtension;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

/// Reads and writes `profile_track_eq`.
pub struct SqliteTrackEqRepository {
    pool: SqlitePool,
}

impl SqliteTrackEqRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl TrackEqRepositoryPort for SqliteTrackEqRepository {
    fn preset_for(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Result<Option<EqPresetId>> {
        let connection = self.pool.get()?;

        let stored: Option<String> = connection
            .query_row(
                "SELECT preset_id FROM profile_track_eq
                 WHERE profile_id = ?1 AND media_file_id = ?2",
                (profile_id.to_string(), media_file_id.to_string()),
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error_in("reading the equaliser chosen for a track"))?;

        stored.map(|id| EqPresetId::parse(&id)).transpose()
    }

    fn remember(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        preset_id: EqPresetId,
        now: Timestamp,
    ) -> Result<()> {
        let connection = self.pool.get()?;

        connection
            .execute(
                "INSERT INTO profile_track_eq
                     (profile_id, media_file_id, preset_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (profile_id, media_file_id)
                 DO UPDATE SET preset_id = excluded.preset_id,
                               updated_at = excluded.updated_at",
                (
                    profile_id.to_string(),
                    media_file_id.to_string(),
                    preset_id.to_string(),
                    now.as_millis(),
                ),
            )
            .map_err(db_error_in("remembering the equaliser for a track"))?;

        Ok(())
    }

    fn forget(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<()> {
        let connection = self.pool.get()?;

        connection
            .execute(
                "DELETE FROM profile_track_eq
                 WHERE profile_id = ?1 AND media_file_id = ?2",
                (profile_id.to_string(), media_file_id.to_string()),
            )
            .map_err(db_error_in("forgetting the equaliser for a track"))?;

        Ok(())
    }
}
