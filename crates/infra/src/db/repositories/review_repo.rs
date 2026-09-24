//! Files waiting for an import decision.

use cadenza_core::Result;
use cadenza_core::domain::ids::{ImportReviewId, MediaFileId, ProfileId};
use cadenza_core::domain::ports::repositories::ImportReviewRepositoryPort;
use cadenza_core::domain::review::{ImportReview, ReviewReason, ReviewState};
use cadenza_core::domain::value_objects::Timestamp;
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, profile_id, media_file_id, duplicate_media_file_id, \
     reason, state, created_at, resolved_at";

/// Reads and writes `import_review`.
pub struct SqliteImportReviewRepository {
    pool: SqlitePool,
}

impl SqliteImportReviewRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl ImportReviewRepositoryPort for SqliteImportReviewRepository {
    fn list_for_profile(
        &self,
        profile_id: ProfileId,
        state: ReviewState,
    ) -> Result<Vec<ImportReview>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM import_review
                 WHERE profile_id = ?1 AND state = ?2
                 ORDER BY created_at"
            ))
            .map_err(db_error_in("listing the review queue"))?;

        let rows = statement
            .query_map((profile_id.to_string(), state.as_str()), ReviewRow::read)
            .map_err(db_error_in("listing the review queue"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing the review queue"))?;

        rows.into_iter().map(ReviewRow::into_domain).collect()
    }

    fn save(&self, review: &ImportReview) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO import_review
                     (id, profile_id, media_file_id, duplicate_media_file_id,
                      reason, state, created_at, resolved_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                     state       = excluded.state,
                     resolved_at = excluded.resolved_at",
                rusqlite::params![
                    review.id.to_string(),
                    review.profile_id.to_string(),
                    review.media_file_id.to_string(),
                    review.duplicate_media_file_id.map(|id| id.to_string()),
                    review.reason.as_str(),
                    review.state.as_str(),
                    review.created_at.as_millis(),
                    review.resolved_at.map(Timestamp::as_millis),
                ],
            )
            .map_err(db_error_in("saving a review entry"))?;
        Ok(())
    }

    fn set_state(&self, id: ImportReviewId, state: ReviewState, now: Timestamp) -> Result<()> {
        let connection = self.pool.get()?;
        // The schema requires resolved_at to be set exactly when the entry leaves
        // the pending state, so the two move together.
        let resolved_at = if state == ReviewState::Pending {
            None
        } else {
            Some(now.as_millis())
        };

        connection
            .execute(
                "UPDATE import_review SET state = ?2, resolved_at = ?3 WHERE id = ?1",
                (id.to_string(), state.as_str(), resolved_at),
            )
            .map_err(db_error_in("resolving a review entry"))?;
        Ok(())
    }
}

/// Column values as stored, before domain validation.
struct ReviewRow {
    id: String,
    profile_id: String,
    media_file_id: String,
    duplicate_media_file_id: Option<String>,
    reason: String,
    state: String,
    created_at: i64,
    resolved_at: Option<i64>,
}

impl ReviewRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            profile_id: row.get("profile_id")?,
            media_file_id: row.get("media_file_id")?,
            duplicate_media_file_id: row.get("duplicate_media_file_id")?,
            reason: row.get("reason")?,
            state: row.get("state")?,
            created_at: row.get("created_at")?,
            resolved_at: row.get("resolved_at")?,
        })
    }

    fn into_domain(self) -> Result<ImportReview> {
        Ok(ImportReview {
            id: ImportReviewId::parse(&self.id)?,
            profile_id: ProfileId::parse(&self.profile_id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            duplicate_media_file_id: self
                .duplicate_media_file_id
                .as_deref()
                .map(MediaFileId::parse)
                .transpose()?,
            reason: ReviewReason::parse(&self.reason)?,
            state: ReviewState::parse(&self.state)?,
            created_at: Timestamp::from_millis(self.created_at),
            resolved_at: self.resolved_at.map(Timestamp::from_millis),
        })
    }
}
