//! Radio sessions and the picks they made.

use cadenza_core::Result;
use cadenza_core::domain::ids::{
    MediaFileId, MoodId, ProfileId, RadioSessionId, RadioSessionItemId,
};
use cadenza_core::domain::radio::{PickReason, RadioFeedback, RadioSession, RadioSessionItem};
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::{CoreError, Result as CoreResult};
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const SESSION_COLUMNS: &str =
    "id, profile_id, mood_id, seed_media_file_id, params_json, created_at, updated_at";

const ITEM_COLUMNS: &str =
    "id, session_id, media_file_id, position, reason_json, created_at, feedback";

/// Reads and writes `radio_sessions` and `radio_session_items`.
pub struct SqliteRadioRepository {
    pool: SqlitePool,
}

impl SqliteRadioRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl cadenza_core::domain::ports::repositories::RadioRepositoryPort for SqliteRadioRepository {
    fn get_session(&self, id: RadioSessionId) -> Result<Option<RadioSession>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM radio_sessions WHERE id = ?1"
            ))
            .map_err(db_error_in("reading a radio session"))?;

        let mut rows = statement
            .query_map((id.to_string(),), SessionRow::read)
            .map_err(db_error_in("reading a radio session"))?;

        match rows.next() {
            None => Ok(None),
            Some(row) => {
                let row = row.map_err(db_error_in("reading a radio session"))?;
                row.into_domain().map(Some)
            }
        }
    }

    fn save_session(&self, session: &RadioSession) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO radio_sessions
                     (id, profile_id, mood_id, seed_media_file_id, params_json,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET
                     mood_id            = excluded.mood_id,
                     seed_media_file_id = excluded.seed_media_file_id,
                     params_json        = excluded.params_json,
                     updated_at         = excluded.updated_at",
                rusqlite::params![
                    session.id.to_string(),
                    session.profile_id.to_string(),
                    session.mood_id.to_string(),
                    session.seed_media_file_id.map(|id| id.to_string()),
                    session.params_json.as_deref(),
                    session.created_at.as_millis(),
                    session.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving a radio session"))?;
        Ok(())
    }

    fn append_item(&self, item: &RadioSessionItem) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO radio_session_items
                     (id, session_id, media_file_id, position, reason_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    item.id.to_string(),
                    item.session_id.to_string(),
                    item.media_file_id.to_string(),
                    item.position,
                    encode(item.reason.as_ref()),
                    item.created_at.as_millis(),
                ],
            )
            .map_err(db_error_in("recording a radio pick"))?;
        Ok(())
    }

    fn recent_items(
        &self,
        session_id: RadioSessionId,
        limit: u32,
    ) -> Result<Vec<RadioSessionItem>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ITEM_COLUMNS} FROM radio_session_items
                  WHERE session_id = ?1
                  ORDER BY position DESC
                  LIMIT ?2"
            ))
            .map_err(db_error_in("reading recent radio picks"))?;

        let rows = statement
            .query_map(
                rusqlite::params![session_id.to_string(), limit],
                ItemRow::read,
            )
            .map_err(db_error_in("reading recent radio picks"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading recent radio picks"))?;

        // Read newest first so the limit takes the newest, handed back oldest
        // first because that is the order the caller reasons about.
        rows.into_iter().rev().map(ItemRow::into_domain).collect()
    }

    fn set_feedback(
        &self,
        session_id: RadioSessionId,
        media_file_id: MediaFileId,
        feedback: RadioFeedback,
        now: Timestamp,
    ) -> Result<()> {
        let connection = self.pool.get()?;

        // The latest pick of that file in that session: a track offered twice
        // was judged the second time.
        let changed = connection
            .execute(
                "UPDATE radio_session_items
                    SET feedback = ?3
                  WHERE id = (
                      SELECT id FROM radio_session_items
                       WHERE session_id = ?1 AND media_file_id = ?2
                       ORDER BY position DESC
                       LIMIT 1
                  )",
                rusqlite::params![
                    session_id.to_string(),
                    media_file_id.to_string(),
                    as_str(feedback),
                ],
            )
            .map_err(db_error_in("recording radio feedback"))?;

        if changed == 0 {
            return Err(CoreError::not_found("radio pick", media_file_id));
        }

        connection
            .execute(
                "UPDATE radio_sessions SET updated_at = ?2 WHERE id = ?1",
                rusqlite::params![session_id.to_string(), now.as_millis()],
            )
            .map_err(db_error_in("recording radio feedback"))?;
        Ok(())
    }

    fn last_offered(&self, profile_id: ProfileId) -> Result<Vec<(MediaFileId, Timestamp)>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(
                "SELECT i.media_file_id, MAX(i.created_at)
                   FROM radio_session_items i
                   JOIN radio_sessions s ON s.id = i.session_id
                  WHERE s.profile_id = ?1
                  GROUP BY i.media_file_id",
            )
            .map_err(db_error_in("reading what radio has played"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(db_error_in("reading what radio has played"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading what radio has played"))?;

        rows.into_iter()
            .map(|(id, at)| Ok((MediaFileId::parse(&id)?, Timestamp::from_millis(at))))
            .collect()
    }

    fn preferences(&self, profile_id: ProfileId) -> Result<Vec<(MediaFileId, f32)>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(
                "SELECT i.media_file_id, i.feedback
                   FROM radio_session_items i
                   JOIN radio_sessions s ON s.id = i.session_id
                  WHERE s.profile_id = ?1 AND i.feedback IS NOT NULL",
            )
            .map_err(db_error_in("reading radio preferences"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error_in("reading radio preferences"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading radio preferences"))?;

        // Summed in Rust rather than in SQL: the weight of a verdict is a
        // domain decision (`RadioFeedback::weight`), and putting the numbers in
        // a CASE expression would be the second place they live.
        let mut totals: Vec<(MediaFileId, f32)> = Vec::new();
        for (media_file_id, verdict) in rows {
            let media_file_id = MediaFileId::parse(&media_file_id)?;
            let weight = parse(&verdict)?.weight();

            match totals
                .iter_mut()
                .find(|(existing, _)| *existing == media_file_id)
            {
                Some((_, total)) => *total += weight,
                None => totals.push((media_file_id, weight)),
            }
        }
        Ok(totals)
    }
}

/// Writes down why a track was picked.
fn encode(reason: Option<&PickReason>) -> Option<String> {
    let reason = reason?;
    Some(
        serde_json::json!({
            "mood": reason.mood,
            "transition": reason.transition,
            "preference": reason.preference,
            "freshness": reason.freshness,
            "repeats_artist": reason.repeats_artist,
            "score": reason.score,
        })
        .to_string(),
    )
}

/// Reads it back, or nothing if it was never written or no longer parses.
fn decode(json: Option<&str>) -> Option<PickReason> {
    let value: serde_json::Value = serde_json::from_str(json?).ok()?;
    let number = |name: &str| value.get(name).and_then(serde_json::Value::as_f64);

    Some(PickReason {
        mood: number("mood")? as f32,
        transition: number("transition")? as f32,
        preference: number("preference")? as f32,
        freshness: number("freshness")? as f32,
        repeats_artist: value
            .get("repeats_artist")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        score: number("score")? as f32,
    })
}

/// The text form stored in `radio_session_items.feedback`.
const fn as_str(feedback: RadioFeedback) -> &'static str {
    match feedback {
        RadioFeedback::Like => "like",
        RadioFeedback::Dislike => "dislike",
        RadioFeedback::Skip => "skip",
    }
}

/// Reads that text form back.
fn parse(text: &str) -> CoreResult<RadioFeedback> {
    match text {
        "like" => Ok(RadioFeedback::Like),
        "dislike" => Ok(RadioFeedback::Dislike),
        "skip" => Ok(RadioFeedback::Skip),
        other => Err(CoreError::invalid(
            "radio feedback",
            format!("unknown verdict {other:?}"),
        )),
    }
}

/// One row of `radio_sessions`.
struct SessionRow {
    id: String,
    profile_id: String,
    mood_id: String,
    seed_media_file_id: Option<String>,
    params_json: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl SessionRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            profile_id: row.get(1)?,
            mood_id: row.get(2)?,
            seed_media_file_id: row.get(3)?,
            params_json: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }

    fn into_domain(self) -> CoreResult<RadioSession> {
        Ok(RadioSession {
            id: RadioSessionId::parse(&self.id)?,
            profile_id: ProfileId::parse(&self.profile_id)?,
            mood_id: MoodId::parse(&self.mood_id)?,
            seed_media_file_id: self
                .seed_media_file_id
                .as_deref()
                .map(MediaFileId::parse)
                .transpose()?,
            params_json: self.params_json,
            created_at: Timestamp::from_millis(self.created_at),
            updated_at: Timestamp::from_millis(self.updated_at),
        })
    }
}

/// One row of `radio_session_items`.
struct ItemRow {
    id: String,
    session_id: String,
    media_file_id: String,
    position: u32,
    reason_json: Option<String>,
    created_at: i64,
}

impl ItemRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            media_file_id: row.get(2)?,
            position: row.get(3)?,
            reason_json: row.get(4)?,
            created_at: row.get(5)?,
        })
    }

    fn into_domain(self) -> CoreResult<RadioSessionItem> {
        Ok(RadioSessionItem {
            id: RadioSessionItemId::parse(&self.id)?,
            session_id: RadioSessionId::parse(&self.session_id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            position: self.position,
            // A reason that cannot be read is not worth failing a session for:
            // it explains a pick that has already been made, and losing the
            // explanation costs a debugging session rather than a listener.
            reason: decode(self.reason_json.as_deref()),
            created_at: Timestamp::from_millis(self.created_at),
        })
    }
}
