//! The background analysis work queue.

use cadenza_core::Result;
use cadenza_core::domain::analysis::{AnalysisJob, AnalysisKind, AnalysisState, MAX_ATTEMPTS};
use cadenza_core::domain::ids::{AnalysisJobId, MediaFileId};
use cadenza_core::domain::policies::analysis_policy::FEATURES_PRIORITY;
use cadenza_core::domain::ports::repositories::AnalysisJobRepositoryPort;
use cadenza_core::domain::value_objects::Timestamp;
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str =
    "id, media_file_id, kind, state, priority, attempts, error, created_at, updated_at";

/// Reads and writes `analysis_jobs`.
pub struct SqliteAnalysisJobRepository {
    pool: SqlitePool,
}

impl SqliteAnalysisJobRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl AnalysisJobRepositoryPort for SqliteAnalysisJobRepository {
    fn enqueue(&self, job: &AnalysisJob) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO analysis_jobs
                     (id, media_file_id, kind, state, priority, attempts, error,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 -- The partial unique index on (media_file_id, kind) for
                 -- outstanding rows is what makes this idempotent: asking twice
                 -- for the same work is not an error, it is the same request.
                 ON CONFLICT DO NOTHING",
                rusqlite::params![
                    job.id.to_string(),
                    job.media_file_id.to_string(),
                    job.kind.as_str(),
                    job.state.as_str(),
                    job.priority,
                    job.attempts,
                    job.error.as_deref(),
                    job.created_at.as_millis(),
                    job.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("enqueueing an analysis job"))?;
        Ok(())
    }

    fn claim_next(
        &self,
        kind: Option<AnalysisKind>,
        now: Timestamp,
    ) -> Result<Option<AnalysisJob>> {
        let connection = self.pool.get()?;

        // Claiming and marking in one statement, so two workers cannot pick up
        // the same job: whoever's UPDATE lands first is the one that gets a row
        // back.
        let mut statement = connection
            .prepare(&format!(
                "UPDATE analysis_jobs
                    SET state = 'running', updated_at = ?1
                  WHERE id = (
                      SELECT id FROM analysis_jobs
                       WHERE state = 'queued'
                         AND (?2 IS NULL OR kind = ?2)
                       ORDER BY priority DESC, created_at
                       LIMIT 1
                  )
              RETURNING {COLUMNS}"
            ))
            .map_err(db_error_in("claiming an analysis job"))?;

        let mut rows = statement
            .query_map(
                rusqlite::params![now.as_millis(), kind.map(AnalysisKind::as_str)],
                JobRow::read,
            )
            .map_err(db_error_in("claiming an analysis job"))?;

        match rows.next() {
            None => Ok(None),
            Some(row) => {
                let row = row.map_err(db_error_in("claiming an analysis job"))?;
                row.into_domain().map(Some)
            }
        }
    }

    fn complete(&self, id: AnalysisJobId, now: Timestamp) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "UPDATE analysis_jobs
                    SET state = 'done', error = NULL, updated_at = ?2
                  WHERE id = ?1",
                rusqlite::params![id.to_string(), now.as_millis()],
            )
            .map_err(db_error_in("completing an analysis job"))?;
        Ok(())
    }

    fn fail(&self, id: AnalysisJobId, error: &str, now: Timestamp) -> Result<()> {
        let connection = self.pool.get()?;

        // Back to the queue while there are attempts left, parked for good once
        // there are not. A file locked by another program deserves another go;
        // a file that is not audio deserves to be left alone.
        connection
            .execute(
                "UPDATE analysis_jobs
                    SET attempts   = attempts + 1,
                        error      = ?2,
                        state      = CASE WHEN attempts + 1 >= ?4 THEN 'failed' ELSE 'queued' END,
                        updated_at = ?3
                  WHERE id = ?1",
                rusqlite::params![id.to_string(), error, now.as_millis(), MAX_ATTEMPTS],
            )
            .map_err(db_error_in("recording an analysis failure"))?;
        Ok(())
    }

    fn pending_count(&self) -> Result<u64> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM analysis_jobs WHERE state IN ('queued', 'running')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error_in("counting outstanding analysis"))
            .map(|count| count.max(0) as u64)
    }

    fn enqueue_missing_features(
        &self,
        extractor_version: &str,
        limit: usize,
        now: Timestamp,
    ) -> Result<u64> {
        let connection = self.pool.get()?;

        // Identifiers are made here rather than in SQLite, which has no UUID of
        // its own, so the candidates are read first and inserted by name.
        let mut statement = connection
            .prepare(
                "SELECT m.id FROM media_files m
                  LEFT JOIN track_features f
                         ON f.media_file_id = m.id AND f.extractor_version = ?1
                  WHERE m.file_state = 'ok'
                    AND f.media_file_id IS NULL
                    -- Neither already waiting nor already given up on. Without
                    -- this a file that cannot be decoded would come back on
                    -- every pass for the rest of the library's life.
                    AND NOT EXISTS (
                        SELECT 1 FROM analysis_jobs j
                         WHERE j.media_file_id = m.id
                           AND j.kind = 'features'
                           AND j.state IN ('queued', 'running', 'failed')
                    )
                  ORDER BY m.created_at
                  LIMIT ?2",
            )
            .map_err(db_error_in("looking for files to analyse"))?;

        let candidates = statement
            .query_map(rusqlite::params![extractor_version, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(db_error_in("looking for files to analyse"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("looking for files to analyse"))?;

        let mut queued = 0;
        for media_file_id in candidates {
            let changed = connection
                .execute(
                    "INSERT INTO analysis_jobs
                         (id, media_file_id, kind, state, priority, attempts,
                          created_at, updated_at)
                     VALUES (?1, ?2, 'features', 'queued', ?3, 0, ?4, ?4)
                     ON CONFLICT DO NOTHING",
                    rusqlite::params![
                        AnalysisJobId::new().to_string(),
                        media_file_id,
                        FEATURES_PRIORITY,
                        now.as_millis(),
                    ],
                )
                .map_err(db_error_in("queueing a file for analysis"))?;
            queued += changed as u64;
        }

        Ok(queued)
    }
}

/// One row of `analysis_jobs`, in the column types SQLite hands back.
struct JobRow {
    id: String,
    media_file_id: String,
    kind: String,
    state: String,
    priority: i32,
    attempts: u8,
    error: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl JobRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            media_file_id: row.get(1)?,
            kind: row.get(2)?,
            state: row.get(3)?,
            priority: row.get(4)?,
            attempts: row.get(5)?,
            error: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    fn into_domain(self) -> Result<AnalysisJob> {
        Ok(AnalysisJob {
            id: AnalysisJobId::parse(&self.id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            kind: AnalysisKind::parse(&self.kind)?,
            state: AnalysisState::parse(&self.state)?,
            priority: self.priority,
            attempts: self.attempts,
            error: self.error,
            created_at: Timestamp::from_millis(self.created_at),
            updated_at: Timestamp::from_millis(self.updated_at),
        })
    }
}
