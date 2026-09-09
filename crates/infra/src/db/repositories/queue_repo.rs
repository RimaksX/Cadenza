//! The queue as it was left.
//!
//! Saving replaces the whole queue rather than diffing it: it is a few dozen
//! rows at most, it changes as a whole every time a track starts, and a diff
//! that gets one lane wrong is a queue that silently plays the wrong thing.

use cadenza_core::domain::ids::{MediaFileId, PlaylistId, ProfileId, RadioSessionId};
use cadenza_core::domain::ports::repositories::QueueRepositoryPort;
use cadenza_core::domain::queue::{Queue, QueueEntry, QueueOrigin, RepeatMode};
use cadenza_core::{CoreError, Result};
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

/// Reads and writes `queue_state` and `queue_entries`.
pub struct SqliteQueueRepository {
    pool: SqlitePool,
}

impl SqliteQueueRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl QueueRepositoryPort for SqliteQueueRepository {
    fn load(&self, profile_id: ProfileId) -> Result<Option<Queue>> {
        let connection = self.pool.get()?;

        let state: Option<(String, i64)> = connection
            .query_row(
                "SELECT repeat_mode, shuffle FROM queue_state WHERE profile_id = ?1",
                (profile_id.to_string(),),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_or_else(
                |err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(db_error_in("reading the saved queue")(other)),
                },
                |row| Ok(Some(row)),
            )?;

        let Some((repeat_mode, shuffle)) = state else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT lane, media_file_id, origin, origin_id
                 FROM queue_entries WHERE profile_id = ?1
                 ORDER BY position",
            )
            .map_err(db_error_in("reading the saved queue"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), EntryRow::read)
            .map_err(db_error_in("reading the saved queue"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading the saved queue"))?;

        let mut queue = Queue::new(profile_id);
        queue.repeat = RepeatMode::parse(&repeat_mode)?;
        queue.shuffle = shuffle != 0;

        for row in rows {
            let lane = row.lane.clone();
            let entry = row.into_domain()?;
            match lane.as_str() {
                "current" => queue.current = Some(entry),
                "manual" => queue.manual.push_back(entry),
                "upcoming" => queue.upcoming.push_back(entry),
                "history" => queue.history.push(entry),
                "round" => queue.round.push(entry),
                other => {
                    return Err(CoreError::invalid(
                        "queue lane",
                        format!("unknown lane {other:?}"),
                    ));
                }
            }
        }

        Ok(Some(queue))
    }

    fn save(&self, queue: &Queue) -> Result<()> {
        let mut connection = self.pool.get()?;
        let transaction = connection
            .transaction()
            .map_err(db_error_in("saving the queue"))?;

        let profile_id = queue.profile_id.to_string();

        transaction
            .execute(
                "INSERT INTO queue_state (profile_id, repeat_mode, shuffle, updated_at)
                 VALUES (?1, ?2, ?3, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
                 ON CONFLICT (profile_id) DO UPDATE SET
                     repeat_mode = excluded.repeat_mode,
                     shuffle     = excluded.shuffle,
                     updated_at  = excluded.updated_at",
                rusqlite::params![profile_id, queue.repeat.as_str(), i64::from(queue.shuffle)],
            )
            .map_err(db_error_in("saving the queue"))?;

        transaction
            .execute(
                "DELETE FROM queue_entries WHERE profile_id = ?1",
                (&profile_id,),
            )
            .map_err(db_error_in("saving the queue"))?;

        {
            let mut insert = transaction
                .prepare(
                    "INSERT INTO queue_entries
                         (profile_id, lane, position, media_file_id, origin, origin_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(db_error_in("saving the queue"))?;

            let lanes = [
                ("current", queue.current.iter().collect::<Vec<_>>()),
                ("manual", queue.manual.iter().collect()),
                ("upcoming", queue.upcoming.iter().collect()),
                ("history", queue.history.iter().collect()),
                ("round", queue.round.iter().collect()),
            ];

            for (lane, entries) in lanes {
                for (position, entry) in entries.into_iter().enumerate() {
                    let (origin, origin_id) = split_origin(entry.origin);
                    insert
                        .execute(rusqlite::params![
                            profile_id,
                            lane,
                            position as i64,
                            entry.media_file_id.to_string(),
                            origin,
                            origin_id,
                        ])
                        .map_err(db_error_in("saving the queue"))?;
                }
            }
        }

        transaction
            .commit()
            .map_err(db_error_in("saving the queue"))?;
        Ok(())
    }

    fn clear(&self, profile_id: ProfileId) -> Result<()> {
        let connection = self.pool.get()?;
        // The entries cascade from the state row.
        connection
            .execute(
                "DELETE FROM queue_state WHERE profile_id = ?1",
                (profile_id.to_string(),),
            )
            .map_err(db_error_in("clearing the queue"))?;
        Ok(())
    }
}

/// The two columns an origin becomes.
fn split_origin(origin: QueueOrigin) -> (&'static str, Option<String>) {
    match origin {
        QueueOrigin::Library => ("library", None),
        QueueOrigin::Playlist(id) => ("playlist", Some(id.to_string())),
        QueueOrigin::Radio(id) => ("radio", Some(id.to_string())),
    }
}

/// Column values as stored, before domain validation.
struct EntryRow {
    lane: String,
    media_file_id: String,
    origin: String,
    origin_id: Option<String>,
}

impl EntryRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            lane: row.get("lane")?,
            media_file_id: row.get("media_file_id")?,
            origin: row.get("origin")?,
            origin_id: row.get("origin_id")?,
        })
    }

    fn into_domain(self) -> Result<QueueEntry> {
        let missing =
            || CoreError::invalid("queue entry", format!("{} without an id", self.origin));

        let origin = match self.origin.as_str() {
            "library" => QueueOrigin::Library,
            "playlist" => {
                QueueOrigin::Playlist(PlaylistId::parse(&self.origin_id.ok_or_else(missing)?)?)
            }
            "radio" => {
                QueueOrigin::Radio(RadioSessionId::parse(&self.origin_id.ok_or_else(missing)?)?)
            }
            other => {
                return Err(CoreError::invalid(
                    "queue entry",
                    format!("unknown origin {other:?}"),
                ));
            }
        };

        Ok(QueueEntry {
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            origin,
        })
    }
}
