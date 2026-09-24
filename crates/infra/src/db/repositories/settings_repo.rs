//! Settings and library folders.
//!
//! This is the only place that knows settings are stored as JSON. The domain
//! deals in [`SettingValue`]; replacing the encoding would not touch `core`.

use std::path::{Path, PathBuf};

use cadenza_core::domain::ids::{ProfileFolderId, ProfileId};
use cadenza_core::domain::ports::repositories::SettingsRepositoryPort;
use cadenza_core::domain::settings::{ProfileFolder, SettingValue};
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Row};
use serde_json::{Number, Value};

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const FOLDER_COLUMNS: &str = "id, profile_id, path, include_subfolders, enabled, last_scan_at";

/// Reads and writes `app_settings`, `profile_settings` and `profile_folders`.
pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl SettingsRepositoryPort for SqliteSettingsRepository {
    fn app_get(&self, key: &str) -> Result<Option<SettingValue>> {
        let connection = self.pool.get()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error_in(&format!("reading the setting {key}")))?;

        stored.as_deref().map(decode).transpose()
    }

    fn app_set(&self, key: &str, value: &SettingValue, now: Timestamp) -> Result<()> {
        let encoded = encode(value)?;
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO app_settings (key, value_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET
                     value_json = excluded.value_json,
                     updated_at = excluded.updated_at",
                (key, encoded, now.as_millis()),
            )
            .map_err(db_error_in(&format!("writing the setting {key}")))?;
        Ok(())
    }

    fn app_remove(&self, key: &str) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute("DELETE FROM app_settings WHERE key = ?1", [key])
            .map_err(db_error_in(&format!("removing the setting {key}")))?;
        Ok(())
    }

    fn profile_get(&self, profile_id: ProfileId, key: &str) -> Result<Option<SettingValue>> {
        let connection = self.pool.get()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT value_json FROM profile_settings WHERE profile_id = ?1 AND key = ?2",
                (profile_id.to_string(), key),
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error_in(&format!("reading the profile setting {key}")))?;

        stored.as_deref().map(decode).transpose()
    }

    fn profile_set(
        &self,
        profile_id: ProfileId,
        key: &str,
        value: &SettingValue,
        now: Timestamp,
    ) -> Result<()> {
        let encoded = encode(value)?;
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO profile_settings (profile_id, key, value_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (profile_id, key) DO UPDATE SET
                     value_json = excluded.value_json,
                     updated_at = excluded.updated_at",
                (profile_id.to_string(), key, encoded, now.as_millis()),
            )
            .map_err(db_error_in(&format!("writing the profile setting {key}")))?;
        Ok(())
    }

    fn profile_remove(&self, profile_id: ProfileId, key: &str) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "DELETE FROM profile_settings WHERE profile_id = ?1 AND key = ?2",
                (profile_id.to_string(), key),
            )
            .map_err(db_error_in(&format!("removing the profile setting {key}")))?;
        Ok(())
    }

    fn list_folders(&self, profile_id: ProfileId) -> Result<Vec<ProfileFolder>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {FOLDER_COLUMNS} FROM profile_folders
                 WHERE profile_id = ?1 ORDER BY path"
            ))
            .map_err(db_error_in("listing library folders"))?;

        let rows = statement
            .query_map([profile_id.to_string()], FolderRow::read)
            .map_err(db_error_in("listing library folders"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing library folders"))?;

        rows.into_iter().map(FolderRow::into_domain).collect()
    }

    fn save_folder(&self, folder: &ProfileFolder) -> Result<()> {
        let path = path_to_sql(&folder.path)?;
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO profile_folders
                     (id, profile_id, path, include_subfolders, enabled, last_scan_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET
                     path               = excluded.path,
                     include_subfolders = excluded.include_subfolders,
                     enabled            = excluded.enabled,
                     last_scan_at       = excluded.last_scan_at",
                (
                    folder.id.to_string(),
                    folder.profile_id.to_string(),
                    path,
                    folder.include_subfolders,
                    folder.enabled,
                    folder.last_scan_at.map(Timestamp::as_millis),
                ),
            )
            .map_err(db_error_in("saving a library folder"))?;
        Ok(())
    }

    fn delete_folder(&self, folder: &ProfileFolder) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "DELETE FROM profile_folders WHERE id = ?1",
                [folder.id.to_string()],
            )
            .map_err(db_error_in("removing a library folder"))?;
        Ok(())
    }
}

/// Encodes a value for `value_json`.
fn encode(value: &SettingValue) -> Result<String> {
    let json =
        match value {
            SettingValue::Bool(flag) => Value::Bool(*flag),
            SettingValue::Integer(number) => Value::Number(Number::from(*number)),
            SettingValue::Float(number) => Number::from_f64(*number)
                .map(Value::Number)
                .ok_or_else(|| {
                    // JSON has no infinity and no NaN. Storing null instead would
                    // turn a broken value into a missing one.
                    CoreError::invalid("setting", format!("{number} has no JSON representation"))
                })?,
            SettingValue::Text(text) => Value::String(text.clone()),
        };
    Ok(json.to_string())
}

/// Decodes a stored `value_json`.
fn decode(stored: &str) -> Result<SettingValue> {
    let json: Value = serde_json::from_str(stored)
        .map_err(|err| CoreError::Storage(format!("a stored setting is not valid JSON: {err}")))?;

    match json {
        Value::Bool(flag) => Ok(SettingValue::Bool(flag)),
        Value::Number(number) => number
            .as_i64()
            .map(SettingValue::Integer)
            .or_else(|| number.as_f64().map(SettingValue::Float))
            .ok_or_else(|| {
                CoreError::Storage(format!(
                    "the stored number {number} does not fit any setting type"
                ))
            }),
        Value::String(text) => Ok(SettingValue::Text(text)),
        // Arrays, objects and null are not settings. A setting that wants
        // structure gets its own table.
        other => Err(CoreError::Storage(format!(
            "a stored setting holds {other}, which is not a scalar"
        ))),
    }
}

/// Converts a path for storage, refusing anything that would not survive.
///
/// `to_string_lossy` would replace an unpaired surrogate with U+FFFD and store a
/// path that no longer points anywhere. Failing loudly beats silently corrupting
/// a library folder.
fn path_to_sql(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        CoreError::invalid(
            "folder path",
            format!("{} is not valid UTF-8 and cannot be stored", path.display()),
        )
    })
}

/// Column values as stored, before domain validation.
struct FolderRow {
    id: String,
    profile_id: String,
    path: String,
    include_subfolders: bool,
    enabled: bool,
    last_scan_at: Option<i64>,
}

impl FolderRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            profile_id: row.get("profile_id")?,
            path: row.get("path")?,
            include_subfolders: row.get("include_subfolders")?,
            enabled: row.get("enabled")?,
            last_scan_at: row.get("last_scan_at")?,
        })
    }

    fn into_domain(self) -> Result<ProfileFolder> {
        Ok(ProfileFolder {
            id: ProfileFolderId::parse(&self.id)?,
            profile_id: ProfileId::parse(&self.profile_id)?,
            path: PathBuf::from(self.path),
            include_subfolders: self.include_subfolders,
            enabled: self.enabled,
            last_scan_at: self.last_scan_at.map(Timestamp::from_millis),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use cadenza_core::domain::settings::SettingValue;

    #[test]
    fn every_scalar_survives_a_round_trip() {
        for value in [
            SettingValue::Bool(true),
            SettingValue::Bool(false),
            SettingValue::Integer(-42),
            SettingValue::Integer(i64::MAX),
            SettingValue::Float(1.5),
            SettingValue::Text("0195c0f2-0000-7000-8000-000000000000".to_owned()),
        ] {
            let encoded = encode(&value).expect("encodable");
            assert_eq!(decode(&encoded).expect("decodable"), value);
        }
    }

    #[test]
    fn text_with_quotes_and_backslashes_survives() {
        let awkward = SettingValue::Text(r#"D:\Music\"Best Of"\"#.to_owned());
        let encoded = encode(&awkward).expect("encodable");
        assert_eq!(decode(&encoded).expect("decodable"), awkward);
    }

    #[test]
    fn values_with_no_json_form_are_refused_rather_than_stored_as_null() {
        assert!(encode(&SettingValue::Float(f64::NAN)).is_err());
        assert!(encode(&SettingValue::Float(f64::INFINITY)).is_err());
    }

    #[test]
    fn a_structured_value_is_not_a_setting() {
        assert!(decode(r#"{"nested": true}"#).is_err());
        assert!(decode("[1, 2, 3]").is_err());
        assert!(decode("null").is_err());
        assert!(decode("not json at all").is_err());
    }

    #[test]
    fn a_whole_float_comes_back_as_an_integer_and_still_reads_as_a_number() {
        // JSON cannot tell 1 from 1.0, so a float setting holding a round number
        // decodes as an integer. `as_float` accepts it; nothing else should care.
        let encoded = encode(&SettingValue::Float(2.0)).expect("encodable");
        let decoded = decode(&encoded).expect("decodable");
        assert!((decoded.as_float().expect("reads as a number") - 2.0).abs() < f64::EPSILON);
    }
}
