//! The global catalogue of physical files.

use std::path::{Path, PathBuf};

use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::MediaFileRepositoryPort;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Row};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, path, file_hash, file_size, file_mtime, format, duration_ms, \
     sample_rate, channels, bitrate, metadata_version, metadata_extracted_at, \
     file_state, created_at, updated_at";

/// Reads and writes `media_files`.
pub struct SqliteMediaFileRepository {
    pool: SqlitePool,
}

impl SqliteMediaFileRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl MediaFileRepositoryPort for SqliteMediaFileRepository {
    fn get(&self, id: MediaFileId) -> Result<Option<MediaFile>> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM media_files WHERE id = ?1"),
                [id.to_string()],
                MediaFileRow::read,
            )
            .optional()
            .map_err(db_error_in("reading a media file"))?
            .map(MediaFileRow::into_domain)
            .transpose()
    }

    fn find_by_path(&self, path: &Path) -> Result<Option<MediaFile>> {
        let text = path_to_sql(path)?;
        let connection = self.pool.get()?;
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM media_files WHERE path = ?1"),
                [text],
                MediaFileRow::read,
            )
            .optional()
            .map_err(db_error_in("looking a media file up by path"))?
            .map(MediaFileRow::into_domain)
            .transpose()
    }

    fn find_by_hash(&self, hash: &str) -> Result<Vec<MediaFile>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM media_files WHERE file_hash = ?1 ORDER BY path"
            ))
            .map_err(db_error_in("looking media files up by hash"))?;

        let rows = statement
            .query_map([hash], MediaFileRow::read)
            .map_err(db_error_in("looking media files up by hash"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("looking media files up by hash"))?;

        rows.into_iter().map(MediaFileRow::into_domain).collect()
    }

    fn save(&self, media_file: &MediaFile) -> Result<()> {
        let path = path_to_sql(&media_file.path)?;
        let properties = &media_file.properties;
        let connection = self.pool.get()?;

        connection
            .execute(
                // `created_at` and `path` stay put on update: a row is identified
                // by its path, and when it was first seen does not change.
                "INSERT INTO media_files
                     (id, path, file_hash, file_size, file_mtime, format, duration_ms,
                      sample_rate, channels, bitrate, metadata_version,
                      metadata_extracted_at, file_state, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT (id) DO UPDATE SET
                     file_hash             = excluded.file_hash,
                     file_size             = excluded.file_size,
                     file_mtime            = excluded.file_mtime,
                     format                = excluded.format,
                     duration_ms           = excluded.duration_ms,
                     sample_rate           = excluded.sample_rate,
                     channels              = excluded.channels,
                     bitrate               = excluded.bitrate,
                     metadata_version      = excluded.metadata_version,
                     metadata_extracted_at = excluded.metadata_extracted_at,
                     file_state            = excluded.file_state,
                     updated_at            = excluded.updated_at",
                rusqlite::params![
                    media_file.id.to_string(),
                    path,
                    media_file.file_hash,
                    signed(media_file.file_size, "file size")?,
                    media_file.file_mtime.as_millis(),
                    media_file.format.as_str(),
                    signed(properties.duration.as_millis(), "duration")?,
                    properties.sample_rate,
                    properties.channels,
                    properties.bitrate,
                    media_file.metadata_version,
                    media_file.metadata_extracted_at.map(Timestamp::as_millis),
                    media_file.state.as_str(),
                    media_file.created_at.as_millis(),
                    media_file.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving a media file"))?;
        Ok(())
    }

    fn set_state(&self, id: MediaFileId, state: FileState, now: Timestamp) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "UPDATE media_files SET file_state = ?2, updated_at = ?3 WHERE id = ?1",
                (id.to_string(), state.as_str(), now.as_millis()),
            )
            .map_err(db_error_in("marking a media file"))?;
        Ok(())
    }
}

/// Converts a path for storage, refusing anything that would not survive.
pub(crate) fn path_to_sql(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        CoreError::invalid(
            "path",
            format!("{} is not valid UTF-8 and cannot be stored", path.display()),
        )
    })
}

/// Column values as stored, before domain validation.
struct MediaFileRow {
    id: String,
    path: String,
    file_hash: Option<String>,
    file_size: i64,
    file_mtime: i64,
    format: String,
    duration_ms: i64,
    sample_rate: i64,
    channels: i64,
    bitrate: Option<i64>,
    metadata_version: Option<String>,
    metadata_extracted_at: Option<i64>,
    file_state: String,
    created_at: i64,
    updated_at: i64,
}

impl MediaFileRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            path: row.get("path")?,
            file_hash: row.get("file_hash")?,
            file_size: row.get("file_size")?,
            file_mtime: row.get("file_mtime")?,
            format: row.get("format")?,
            duration_ms: row.get("duration_ms")?,
            sample_rate: row.get("sample_rate")?,
            channels: row.get("channels")?,
            bitrate: row.get("bitrate")?,
            metadata_version: row.get("metadata_version")?,
            metadata_extracted_at: row.get("metadata_extracted_at")?,
            file_state: row.get("file_state")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    fn into_domain(self) -> Result<MediaFile> {
        Ok(MediaFile {
            id: MediaFileId::parse(&self.id)?,
            path: PathBuf::from(self.path),
            file_hash: self.file_hash,
            file_size: cast(self.file_size, "file size")?,
            file_mtime: Timestamp::from_millis(self.file_mtime),
            format: AudioFormat::parse(&self.format)?,
            properties: AudioProperties {
                duration: DurationMs::from_millis(cast(self.duration_ms, "duration")?),
                sample_rate: cast(self.sample_rate, "sample rate")?,
                channels: cast(self.channels, "channel count")?,
                bitrate: self
                    .bitrate
                    .map(|value| cast(value, "bitrate"))
                    .transpose()?,
            },
            metadata_version: self.metadata_version,
            metadata_extracted_at: self.metadata_extracted_at.map(Timestamp::from_millis),
            state: FileState::parse(&self.file_state)?,
            created_at: Timestamp::from_millis(self.created_at),
            updated_at: Timestamp::from_millis(self.updated_at),
        })
    }
}

/// Widens a domain `u64` into the signed integer SQLite stores.
///
/// rusqlite refuses `u64` on purpose: SQLite integers are signed 64-bit, so the
/// top bit has nowhere to go. Converting explicitly means a file larger than
/// eight exabytes is reported rather than silently stored as a negative size.
fn signed(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| CoreError::invalid(field, format!("{value} is too large")))
}

/// Narrows a stored integer into the domain's type.
///
/// The schema already rejects negatives and zeroes where they are impossible, so
/// a failure here means the file was written by something else.
fn cast<T: TryFrom<i64>>(value: i64, field: &'static str) -> Result<T> {
    T::try_from(value).map_err(|_| CoreError::invalid(field, format!("{value} is out of range")))
}
