//! Playlists and their entries.
//!
//! Entries are replaced wholesale rather than edited one at a time, because
//! reordering renumbers many rows at once: migration 6 deliberately leaves
//! `(playlist_id, position)` without a unique index so that one transaction can
//! pass through the states a row-by-row renumber would be caught in.

use cadenza_core::Result;
use cadenza_core::domain::ids::{MediaFileId, PlaylistId, PlaylistItemId, ProfileId};
use cadenza_core::domain::playlist::{Playlist, PlaylistItem};
use cadenza_core::domain::ports::repositories::PlaylistRepositoryPort;
use cadenza_core::domain::value_objects::Timestamp;
use rusqlite::{OptionalExtension, Row};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, profile_id, name, description, is_smart, rule_json, \
     created_at, updated_at";

/// Reads and writes `playlists` and `playlist_items`.
pub struct SqlitePlaylistRepository {
    pool: SqlitePool,
}

impl SqlitePlaylistRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl PlaylistRepositoryPort for SqlitePlaylistRepository {
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<Playlist>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM playlists
                 WHERE profile_id = ?1
                 ORDER BY name COLLATE NOCASE"
            ))
            .map_err(db_error_in("listing playlists"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), PlaylistRow::read)
            .map_err(db_error_in("listing playlists"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing playlists"))?;

        rows.into_iter().map(PlaylistRow::into_domain).collect()
    }

    fn get(&self, id: PlaylistId) -> Result<Option<Playlist>> {
        let connection = self.pool.get()?;
        let row = connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM playlists WHERE id = ?1"),
                (id.to_string(),),
                PlaylistRow::read,
            )
            .optional()
            .map_err(db_error_in("reading a playlist"))?;

        row.map(PlaylistRow::into_domain).transpose()
    }

    fn save(&self, playlist: &Playlist) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO playlists
                     (id, profile_id, name, description, is_smart, rule_json,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                     name        = excluded.name,
                     description = excluded.description,
                     is_smart    = excluded.is_smart,
                     rule_json   = excluded.rule_json,
                     updated_at  = excluded.updated_at",
                rusqlite::params![
                    playlist.id.to_string(),
                    playlist.profile_id.to_string(),
                    playlist.name,
                    playlist.description,
                    i64::from(playlist.is_smart),
                    playlist.rule_json,
                    playlist.created_at.as_millis(),
                    playlist.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving a playlist"))?;
        Ok(())
    }

    fn delete(&self, id: PlaylistId) -> Result<()> {
        let connection = self.pool.get()?;
        // The entries cascade; the tracks themselves are untouched.
        connection
            .execute("DELETE FROM playlists WHERE id = ?1", (id.to_string(),))
            .map_err(db_error_in("deleting a playlist"))?;
        Ok(())
    }

    fn items(&self, playlist_id: PlaylistId) -> Result<Vec<PlaylistItem>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(
                "SELECT id, playlist_id, media_file_id, position, added_at, by_hand
                 FROM playlist_items WHERE playlist_id = ?1
                 ORDER BY position",
            )
            .map_err(db_error_in("reading a playlist's entries"))?;

        let rows = statement
            .query_map((playlist_id.to_string(),), ItemRow::read)
            .map_err(db_error_in("reading a playlist's entries"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("reading a playlist's entries"))?;

        rows.into_iter().map(ItemRow::into_domain).collect()
    }

    fn replace_items(&self, playlist_id: PlaylistId, items: &[PlaylistItem]) -> Result<()> {
        let mut connection = self.pool.get()?;
        let transaction = connection
            .transaction()
            .map_err(db_error_in("rewriting a playlist"))?;

        transaction
            .execute(
                "DELETE FROM playlist_items WHERE playlist_id = ?1",
                (playlist_id.to_string(),),
            )
            .map_err(db_error_in("rewriting a playlist"))?;

        {
            let mut insert = transaction
                .prepare(
                    "INSERT INTO playlist_items
                         (id, playlist_id, media_file_id, position, added_at, by_hand)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(db_error_in("rewriting a playlist"))?;

            for item in items {
                insert
                    .execute(rusqlite::params![
                        item.id.to_string(),
                        // The caller's playlist, not the item's: an entry moved
                        // between playlists would otherwise be written back to
                        // the one it came from.
                        playlist_id.to_string(),
                        item.media_file_id.to_string(),
                        i64::from(item.position),
                        item.added_at.as_millis(),
                        i64::from(item.by_hand),
                    ])
                    .map_err(db_error_in("rewriting a playlist"))?;
            }
        }

        transaction
            .commit()
            .map_err(db_error_in("rewriting a playlist"))?;
        Ok(())
    }

    fn delete_item(&self, id: PlaylistItemId) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "DELETE FROM playlist_items WHERE id = ?1",
                (id.to_string(),),
            )
            .map_err(db_error_in("removing a playlist entry"))?;
        Ok(())
    }
}

/// Column values as stored, before domain validation.
struct PlaylistRow {
    id: String,
    profile_id: String,
    name: String,
    description: Option<String>,
    is_smart: i64,
    rule_json: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl PlaylistRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            profile_id: row.get("profile_id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            is_smart: row.get("is_smart")?,
            rule_json: row.get("rule_json")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn into_domain(self) -> Result<Playlist> {
        Ok(Playlist {
            id: PlaylistId::parse(&self.id)?,
            profile_id: ProfileId::parse(&self.profile_id)?,
            name: self.name,
            description: self.description,
            is_smart: self.is_smart != 0,
            rule_json: self.rule_json,
            created_at: Timestamp::from_millis(self.created_at),
            updated_at: Timestamp::from_millis(self.updated_at),
        })
    }
}

struct ItemRow {
    id: String,
    playlist_id: String,
    media_file_id: String,
    position: i64,
    added_at: i64,
    by_hand: i64,
}

impl ItemRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            playlist_id: row.get("playlist_id")?,
            media_file_id: row.get("media_file_id")?,
            position: row.get("position")?,
            added_at: row.get("added_at")?,
            by_hand: row.get("by_hand")?,
        })
    }

    fn into_domain(self) -> Result<PlaylistItem> {
        Ok(PlaylistItem {
            id: PlaylistItemId::parse(&self.id)?,
            playlist_id: PlaylistId::parse(&self.playlist_id)?,
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            // The column is constrained to be non-negative, so a value that does
            // not fit is a database written by something else.
            position: u32::try_from(self.position).unwrap_or(0),
            added_at: Timestamp::from_millis(self.added_at),
            by_hand: self.by_hand != 0,
        })
    }
}
