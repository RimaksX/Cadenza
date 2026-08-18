//! Listening events, and the counting done over them.
//!
//! Two ports in one adapter because they are two questions about one table:
//! what happened, and what it adds up to. Splitting them across two files would
//! mean two copies of the same column list and the same outcome mapping.

use cadenza_core::Result;
use cadenza_core::domain::ids::{MediaFileId, PlayEventId, ProfileId, RadioSessionId};
use cadenza_core::domain::ports::repositories::{PlayEventRepositoryPort, StatsRepositoryPort};
use cadenza_core::domain::stats::{ListeningSummary, PlayEvent, PlayOutcome, PlaySource};
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_core::{CoreError, Result as CoreResult};
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, profile_id, media_file_id, source, radio_session_id, \
     started_at, ended_at, played_ms, duration_ms, completed, skipped";

/// Reads and writes `play_events`.
pub struct SqliteHistoryRepository {
    pool: SqlitePool,
}

impl SqliteHistoryRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl PlayEventRepositoryPort for SqliteHistoryRepository {
    fn append(&self, event: &PlayEvent) -> Result<()> {
        // The two flags rather than the enum: the schema keeps them separate and
        // forbids their contradiction, and the domain keeps them together so the
        // contradiction cannot be written in the first place.
        let (completed, skipped) = match event.outcome {
            PlayOutcome::Completed => (1, 0),
            PlayOutcome::Skipped => (0, 1),
            PlayOutcome::Partial => (0, 0),
        };

        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO play_events
                     (id, profile_id, media_file_id, source, radio_session_id,
                      started_at, ended_at, played_ms, duration_ms, completed, skipped)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    event.id.to_string(),
                    event.profile_id.to_string(),
                    event.media_file_id.to_string(),
                    event.source.as_str(),
                    event.radio_session_id.map(|id| id.to_string()),
                    event.started_at.as_millis(),
                    event.ended_at.map(Timestamp::as_millis),
                    event.played.as_millis() as i64,
                    event.duration.as_millis() as i64,
                    completed,
                    skipped,
                ],
            )
            .map_err(db_error_in("recording a listen"))?;
        Ok(())
    }

    fn recent(
        &self,
        profile_id: ProfileId,
        since: Timestamp,
        limit: u32,
    ) -> Result<Vec<PlayEvent>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM play_events
                  WHERE profile_id = ?1 AND started_at >= ?2
                  ORDER BY started_at DESC
                  LIMIT ?3"
            ))
            .map_err(db_error_in("reading recent listens"))?;

        let rows = statement
            .query_map(
                rusqlite::params![profile_id.to_string(), since.as_millis(), limit],
                EventRow::read,
            )
            .map_err(db_error_in("reading recent listens"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading recent listens"))?;

        rows.into_iter().map(EventRow::into_domain).collect()
    }

    fn purge_before(&self, profile_id: ProfileId, cutoff: Timestamp) -> Result<u64> {
        let connection = self.pool.get()?;
        let removed = connection
            .execute(
                "DELETE FROM play_events WHERE profile_id = ?1 AND started_at < ?2",
                rusqlite::params![profile_id.to_string(), cutoff.as_millis()],
            )
            .map_err(db_error_in("purging old listens"))?;
        Ok(removed as u64)
    }

    fn purge_all(&self, profile_id: ProfileId) -> Result<u64> {
        let connection = self.pool.get()?;
        let removed = connection
            .execute(
                "DELETE FROM play_events WHERE profile_id = ?1",
                (profile_id.to_string(),),
            )
            .map_err(db_error_in("forgetting a profile's history"))?;
        Ok(removed as u64)
    }
}

impl StatsRepositoryPort for SqliteHistoryRepository {
    fn summary(&self, profile_id: ProfileId, since: Timestamp) -> Result<ListeningSummary> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(completed), 0),
                        COALESCE(SUM(skipped), 0),
                        COUNT(DISTINCT media_file_id),
                        COALESCE(SUM(played_ms), 0)
                   FROM play_events
                  WHERE profile_id = ?1 AND started_at >= ?2",
                rusqlite::params![profile_id.to_string(), since.as_millis()],
                |row| {
                    Ok(ListeningSummary {
                        started: row.get::<_, i64>(0)?.max(0) as u32,
                        completed: row.get::<_, i64>(1)?.max(0) as u32,
                        skipped: row.get::<_, i64>(2)?.max(0) as u32,
                        tracks: row.get::<_, i64>(3)?.max(0) as u32,
                        listened: DurationMs::from_millis(row.get::<_, i64>(4)?.max(0) as u64),
                    })
                },
            )
            .map_err(db_error_in("summing up a month of listening"))
    }

    fn top_tracks(
        &self,
        profile_id: ProfileId,
        since: Timestamp,
        limit: u32,
    ) -> Result<Vec<(MediaFileId, u32)>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(
                // Completed listens only. A track skipped forty times is not a
                // favourite, and counting every start would make the top of the
                // list a list of what annoyed the listener most.
                "SELECT media_file_id, COUNT(*) AS plays
                   FROM play_events
                  WHERE profile_id = ?1 AND started_at >= ?2 AND completed = 1
                  GROUP BY media_file_id
                  ORDER BY plays DESC, media_file_id
                  LIMIT ?3",
            )
            .map_err(db_error_in("counting top tracks"))?;

        let rows = statement
            .query_map(
                rusqlite::params![profile_id.to_string(), since.as_millis(), limit],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(db_error_in("counting top tracks"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("counting top tracks"))?;

        rows.into_iter()
            .map(|(id, plays)| Ok((MediaFileId::parse(&id)?, plays.max(0) as u32)))
            .collect()
    }
}

/// One row of `play_events`, in the column types SQLite hands back.
struct EventRow {
    id: String,
    profile_id: String,
    media_file_id: String,
    source: String,
    radio_session_id: Option<String>,
    started_at: i64,
    ended_at: Option<i64>,
    played_ms: i64,
    duration_ms: i64,
    completed: i64,
    skipped: i64,
}

impl EventRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            profile_id: row.get(1)?,
            media_file_id: row.get(2)?,
            source: row.get(3)?,
            radio_session_id: row.get(4)?,
            started_at: row.get(5)?,
            ended_at: row.get(6)?,
            played_ms: row.get(7)?,
            duration_ms: row.get(8)?,
            completed: row.get(9)?,
            skipped: row.get(10)?,
        })
    }

    fn into_domain(self) -> CoreResult<PlayEvent> {
        let outcome = match (self.completed != 0, self.skipped != 0) {
            (true, false) => PlayOutcome::Completed,
            (false, true) => PlayOutcome::Skipped,
            (false, false) => PlayOutcome::Partial,
            // The schema forbids it, so a row like this was written by
            // something else. Refusing beats picking one of the two.
            (true, true) => {
                return Err(CoreError::invalid(
                    "play event",
                    "a listen cannot be both completed and skipped",
                ));
            }
        };

        Ok(PlayEvent {
            id: PlayEventId::parse(&self.id)?,
            profile_id: ProfileId::parse(&self.profile_id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            source: PlaySource::parse(&self.source)?,
            radio_session_id: self
                .radio_session_id
                .as_deref()
                .map(RadioSessionId::parse)
                .transpose()?,
            started_at: Timestamp::from_millis(self.started_at),
            ended_at: self.ended_at.map(Timestamp::from_millis),
            played: DurationMs::from_millis(self.played_ms.max(0) as u64),
            duration: DurationMs::from_millis(self.duration_ms.max(0) as u64),
            outcome,
        })
    }
}
