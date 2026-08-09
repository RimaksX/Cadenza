//! Artists, albums and genres.
//!
//! Three small adapters in one file. PROJECT_MASTER section 5 gives each its own
//! (`artist_repo.rs`, `album_repo.rs`, `genre_repo.rs`), but each is a dozen
//! lines of the same shape over the same three-table corner of the schema, and
//! they are always changed together. Splitting them would be three files of
//! imports for no reader's benefit.

use cadenza_core::Result;
use cadenza_core::domain::album::Album;
use cadenza_core::domain::artist::Artist;
use cadenza_core::domain::genre::Genre;
use cadenza_core::domain::ids::{AlbumId, ArtistId, GenreId, MediaFileId};
use cadenza_core::domain::ports::repositories::{
    AlbumRepositoryPort, ArtistRepositoryPort, GenreRepositoryPort,
};
use cadenza_core::domain::value_objects::Timestamp;
use rusqlite::{OptionalExtension, Row};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

/// Reads and writes `artists`.
pub struct SqliteArtistRepository {
    pool: SqlitePool,
}

impl SqliteArtistRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl ArtistRepositoryPort for SqliteArtistRepository {
    fn get(&self, id: ArtistId) -> Result<Option<Artist>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT id, name, sort_name, created_at, updated_at FROM artists WHERE id = ?1",
                [id.to_string()],
                read_artist,
            )
            .optional()
            .map_err(db_error_in("reading an artist"))?
            .transpose()
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Artist>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT id, name, sort_name, created_at, updated_at FROM artists WHERE name = ?1",
                [name],
                read_artist,
            )
            .optional()
            .map_err(db_error_in("looking an artist up by name"))?
            .transpose()
    }

    fn save(&self, artist: &Artist) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO artists (id, name, sort_name, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                     name       = excluded.name,
                     sort_name  = excluded.sort_name,
                     updated_at = excluded.updated_at",
                (
                    artist.id.to_string(),
                    &artist.name,
                    &artist.sort_name,
                    artist.created_at.as_millis(),
                    artist.updated_at.as_millis(),
                ),
            )
            .map_err(db_error_in("saving an artist"))?;
        Ok(())
    }
}

fn read_artist(row: &Row<'_>) -> rusqlite::Result<Result<Artist>> {
    let id: String = row.get("id")?;
    let name: String = row.get("name")?;
    let sort_name: String = row.get("sort_name")?;
    let created_at: i64 = row.get("created_at")?;
    let updated_at: i64 = row.get("updated_at")?;

    Ok(ArtistId::parse(&id).map(|id| Artist {
        id,
        name,
        sort_name,
        created_at: Timestamp::from_millis(created_at),
        updated_at: Timestamp::from_millis(updated_at),
    }))
}

/// Reads and writes `albums`.
pub struct SqliteAlbumRepository {
    pool: SqlitePool,
}

impl SqliteAlbumRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl AlbumRepositoryPort for SqliteAlbumRepository {
    fn get(&self, id: AlbumId) -> Result<Option<Album>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT id, artist_id, title, year, created_at, updated_at
                 FROM albums WHERE id = ?1",
                [id.to_string()],
                read_album,
            )
            .optional()
            .map_err(db_error_in("reading an album"))?
            .transpose()
    }

    fn find(&self, title: &str, artist_id: Option<ArtistId>) -> Result<Option<Album>> {
        let connection = self.pool.get()?;
        connection
            // IFNULL on both sides: NULL never equals NULL, so a compilation
            // with no album artist would otherwise never be found again and a
            // second copy would be created on every scan.
            .query_row(
                "SELECT id, artist_id, title, year, created_at, updated_at
                 FROM albums WHERE title = ?1 AND IFNULL(artist_id, '') = IFNULL(?2, '')",
                (title, artist_id.map(|id| id.to_string())),
                read_album,
            )
            .optional()
            .map_err(db_error_in("looking an album up"))?
            .transpose()
    }

    fn save(&self, album: &Album) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO albums (id, artist_id, title, year, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     artist_id  = excluded.artist_id,
                     title      = excluded.title,
                     year       = excluded.year,
                     updated_at = excluded.updated_at",
                (
                    album.id.to_string(),
                    album.artist_id.map(|id| id.to_string()),
                    &album.title,
                    album.year,
                    album.created_at.as_millis(),
                    album.updated_at.as_millis(),
                ),
            )
            .map_err(db_error_in("saving an album"))?;
        Ok(())
    }
}

fn read_album(row: &Row<'_>) -> rusqlite::Result<Result<Album>> {
    let id: String = row.get("id")?;
    let artist_id: Option<String> = row.get("artist_id")?;
    let title: String = row.get("title")?;
    let year: Option<i64> = row.get("year")?;
    let created_at: i64 = row.get("created_at")?;
    let updated_at: i64 = row.get("updated_at")?;

    Ok((|| {
        Ok(Album {
            id: AlbumId::parse(&id)?,
            artist_id: artist_id.as_deref().map(ArtistId::parse).transpose()?,
            title,
            year: year.and_then(|value| u16::try_from(value).ok()),
            created_at: Timestamp::from_millis(created_at),
            updated_at: Timestamp::from_millis(updated_at),
        })
    })())
}

/// Reads and writes `genres` and `track_genres`.
pub struct SqliteGenreRepository {
    pool: SqlitePool,
}

impl SqliteGenreRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl GenreRepositoryPort for SqliteGenreRepository {
    fn find_by_name(&self, name: &str) -> Result<Option<Genre>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT id, name FROM genres WHERE name = ?1",
                [name],
                read_genre,
            )
            .optional()
            .map_err(db_error_in("looking a genre up"))?
            .transpose()
    }

    fn save(&self, genre: &Genre) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO genres (id, name) VALUES (?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET name = excluded.name",
                (genre.id.to_string(), &genre.name),
            )
            .map_err(db_error_in("saving a genre"))?;
        Ok(())
    }

    fn for_media_file(&self, media_file_id: MediaFileId) -> Result<Vec<Genre>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(
                "SELECT g.id, g.name FROM genres g
                 JOIN track_genres tg ON tg.genre_id = g.id
                 WHERE tg.media_file_id = ?1
                 ORDER BY g.name",
            )
            .map_err(db_error_in("listing genres for a file"))?;

        let rows = statement
            .query_map([media_file_id.to_string()], read_genre)
            .map_err(db_error_in("listing genres for a file"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing genres for a file"))?;

        rows.into_iter().collect()
    }

    fn set_for_media_file(&self, media_file_id: MediaFileId, genres: &[GenreId]) -> Result<()> {
        let mut connection = self.pool.get()?;
        // One transaction: a file briefly having no genres, or having both the
        // old and new sets, would be visible to a concurrent reader in WAL mode.
        let transaction = connection
            .transaction()
            .map_err(db_error_in("replacing the genres of a file"))?;

        transaction
            .execute(
                "DELETE FROM track_genres WHERE media_file_id = ?1",
                [media_file_id.to_string()],
            )
            .map_err(db_error_in("replacing the genres of a file"))?;

        for genre_id in genres {
            transaction
                .execute(
                    "INSERT INTO track_genres (media_file_id, genre_id) VALUES (?1, ?2)",
                    (media_file_id.to_string(), genre_id.to_string()),
                )
                .map_err(db_error_in("replacing the genres of a file"))?;
        }

        transaction
            .commit()
            .map_err(db_error_in("replacing the genres of a file"))
    }
}

fn read_genre(row: &Row<'_>) -> rusqlite::Result<Result<Genre>> {
    let id: String = row.get("id")?;
    let name: String = row.get("name")?;
    Ok(GenreId::parse(&id).map(|id| Genre { id, name }))
}
