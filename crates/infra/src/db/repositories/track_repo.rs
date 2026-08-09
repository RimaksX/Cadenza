//! Per-profile library membership.

use cadenza_core::domain::ids::{AlbumId, ArtistId, MediaFileId, ProfileId};
use cadenza_core::domain::ports::repositories::TrackRepositoryPort;
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Row};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "profile_id, media_file_id, title, artist_id, album_id, \
     track_no, disc_no, year, added_at, removed_at";

/// Reads and writes `profile_tracks`.
pub struct SqliteTrackRepository {
    pool: SqlitePool,
}

impl SqliteTrackRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl TrackRepositoryPort for SqliteTrackRepository {
    fn get(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<Option<Track>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM profile_tracks
                     WHERE profile_id = ?1 AND media_file_id = ?2"
                ),
                (profile_id.to_string(), media_file_id.to_string()),
                TrackRow::read,
            )
            .optional()
            .map_err(db_error_in("reading a track"))?
            .map(TrackRow::into_domain)
            .transpose()
    }

    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<Track>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            // Tombstones are excluded: a removed track is not in the library,
            // even though the row survives to keep the listener's edits.
            .prepare(&format!(
                "SELECT {COLUMNS} FROM profile_tracks
                 WHERE profile_id = ?1 AND removed_at IS NULL
                 ORDER BY title COLLATE NOCASE"
            ))
            .map_err(db_error_in("listing a library"))?;

        let rows = statement
            .query_map([profile_id.to_string()], TrackRow::read)
            .map_err(db_error_in("listing a library"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing a library"))?;

        rows.into_iter().map(TrackRow::into_domain).collect()
    }

    fn save(&self, track: &Track) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                // `added_at` survives an update: re-importing a file the listener
                // already had does not make it newly added.
                "INSERT INTO profile_tracks
                     (profile_id, media_file_id, title, artist_id, album_id,
                      track_no, disc_no, year, added_at, removed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (profile_id, media_file_id) DO UPDATE SET
                     title      = excluded.title,
                     artist_id  = excluded.artist_id,
                     album_id   = excluded.album_id,
                     track_no   = excluded.track_no,
                     disc_no    = excluded.disc_no,
                     year       = excluded.year,
                     removed_at = excluded.removed_at",
                rusqlite::params![
                    track.profile_id.to_string(),
                    track.media_file_id.to_string(),
                    track.title,
                    track.artist_id.map(|id| id.to_string()),
                    track.album_id.map(|id| id.to_string()),
                    track.track_no,
                    track.disc_no,
                    track.year,
                    track.added_at.as_millis(),
                    track.removed_at.map(Timestamp::as_millis),
                ],
            )
            .map_err(db_error_in("saving a track"))?;
        Ok(())
    }

    fn remove(
        &self,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
        now: Timestamp,
    ) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            // A tombstone, not a delete. The file stays on disk and in the
            // catalogue, other profiles keep their copy, and the listener's own
            // title and artist edits are still here if they add it back
            // (PROJECT_MASTER 2.1).
            .execute(
                "UPDATE profile_tracks SET removed_at = ?3
                 WHERE profile_id = ?1 AND media_file_id = ?2 AND removed_at IS NULL",
                (
                    profile_id.to_string(),
                    media_file_id.to_string(),
                    now.as_millis(),
                ),
            )
            .map_err(db_error_in("removing a track from a library"))?;
        Ok(())
    }

    fn restore(&self, profile_id: ProfileId, media_file_id: MediaFileId) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "UPDATE profile_tracks SET removed_at = NULL
                 WHERE profile_id = ?1 AND media_file_id = ?2",
                (profile_id.to_string(), media_file_id.to_string()),
            )
            .map_err(db_error_in("restoring a track"))?;
        Ok(())
    }
}

/// Column values as stored, before domain validation.
struct TrackRow {
    profile_id: String,
    media_file_id: String,
    title: String,
    artist_id: Option<String>,
    album_id: Option<String>,
    track_no: Option<i64>,
    disc_no: Option<i64>,
    year: Option<i64>,
    added_at: i64,
    removed_at: Option<i64>,
}

impl TrackRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            profile_id: row.get("profile_id")?,
            media_file_id: row.get("media_file_id")?,
            title: row.get("title")?,
            artist_id: row.get("artist_id")?,
            album_id: row.get("album_id")?,
            track_no: row.get("track_no")?,
            disc_no: row.get("disc_no")?,
            year: row.get("year")?,
            added_at: row.get("added_at")?,
            removed_at: row.get("removed_at")?,
        })
    }

    fn into_domain(self) -> Result<Track> {
        Ok(Track {
            profile_id: ProfileId::parse(&self.profile_id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            title: self.title,
            artist_id: self.artist_id.as_deref().map(ArtistId::parse).transpose()?,
            album_id: self.album_id.as_deref().map(AlbumId::parse).transpose()?,
            track_no: small(self.track_no, "track number")?,
            disc_no: small(self.disc_no, "disc number")?,
            year: small(self.year, "year")?,
            added_at: Timestamp::from_millis(self.added_at),
            removed_at: self.removed_at.map(Timestamp::from_millis),
        })
    }
}

fn small(value: Option<i64>, field: &'static str) -> Result<Option<u16>> {
    value
        .map(|number| {
            u16::try_from(number)
                .map_err(|_| CoreError::invalid(field, format!("{number} is out of range")))
        })
        .transpose()
}
