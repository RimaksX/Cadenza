//! Equaliser presets, built in and the listener's own.
//!
//! The parametric bands live in one JSON column rather than a table of their
//! own. That is the exception the schema allows itself and it is worth naming:
//! eight bands are read and written together, always, and are never queried,
//! filtered or joined on — nobody asks "which presets have a bell at 4 kHz".
//! What made `profiles.settings_json` wrong was that its contents *were*
//! questions somebody wanted to ask.
//!
//! Every band is validated on the way out (`eq_policy::validate_advanced_bands`)
//! rather than trusted: these numbers reach a realtime filter, and this column
//! is the one place a hand-edited file could put a Q of zero into one.

use cadenza_core::domain::eq::{EqBand, EqMode, EqPreset, SimpleEq};
use cadenza_core::domain::ids::{EqPresetId, ProfileId};
use cadenza_core::domain::policies::eq_policy::{default_advanced_bands, validate_advanced_bands};
use cadenza_core::domain::ports::repositories::EqPresetRepositoryPort;
use cadenza_core::domain::value_objects::{GainDb, Timestamp};
use cadenza_core::{CoreError, Result};
use rusqlite::{OptionalExtension, Row};
use serde_json::Value;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, profile_id, name, is_builtin, mode, \
     simple_bass_gain, simple_mid_gain, simple_treble_gain, \
     advanced_bands_json, created_at, updated_at";

/// Reads and writes `eq_presets`.
pub struct SqliteEqPresetRepository {
    pool: SqlitePool,
}

impl SqliteEqPresetRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl EqPresetRepositoryPort for SqliteEqPresetRepository {
    /// The built-ins first, in the order PROJECT_MASTER 2.8 names them, then
    /// the listener's own by name.
    ///
    /// The order of the built-ins is their identifiers': migration 15 numbers
    /// them the way the specification lists them, which is an order with a
    /// shape — Flat, then the genres, then the two boosts — and alphabetical is
    /// not.
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<EqPreset>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM eq_presets
                 WHERE profile_id IS NULL OR profile_id = ?1
                 ORDER BY is_builtin DESC,
                          CASE WHEN is_builtin = 1 THEN id END,
                          name COLLATE NOCASE"
            ))
            .map_err(db_error_in("listing equaliser presets"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), read)
            .map_err(db_error_in("listing equaliser presets"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing equaliser presets"))?;

        rows.into_iter().map(into_domain).collect()
    }

    fn get(&self, id: EqPresetId) -> Result<Option<EqPreset>> {
        let connection = self.pool.get()?;
        let row = connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM eq_presets WHERE id = ?1"),
                (id.to_string(),),
                read,
            )
            .optional()
            .map_err(db_error_in("reading an equaliser preset"))?;

        row.map(into_domain).transpose()
    }

    fn save(&self, preset: &EqPreset) -> Result<()> {
        if preset.is_builtin {
            return Err(CoreError::invalid(
                "eq preset",
                "a built-in preset cannot be edited",
            ));
        }
        let profile_id = preset.profile_id.ok_or_else(|| {
            CoreError::invalid(
                "eq preset",
                "a preset of the listener's own belongs to them",
            )
        })?;

        if preset.mode == EqMode::Advanced {
            validate_advanced_bands(&preset.advanced)?;
        }

        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO eq_presets
                     (id, profile_id, name, is_builtin, mode,
                      simple_bass_gain, simple_mid_gain, simple_treble_gain,
                      advanced_bands_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name,
                     mode = excluded.mode,
                     simple_bass_gain = excluded.simple_bass_gain,
                     simple_mid_gain = excluded.simple_mid_gain,
                     simple_treble_gain = excluded.simple_treble_gain,
                     advanced_bands_json = excluded.advanced_bands_json,
                     updated_at = excluded.updated_at",
                rusqlite::params![
                    preset.id.to_string(),
                    profile_id.to_string(),
                    preset.name,
                    preset.mode.as_str(),
                    f64::from(preset.simple.bass.as_db()),
                    f64::from(preset.simple.mid.as_db()),
                    f64::from(preset.simple.treble.as_db()),
                    encode_bands(&preset.advanced),
                    preset.created_at.as_millis(),
                    preset.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving an equaliser preset"))?;

        Ok(())
    }

    fn delete(&self, id: EqPresetId) -> Result<()> {
        let connection = self.pool.get()?;
        let removed = connection
            .execute(
                "DELETE FROM eq_presets WHERE id = ?1 AND is_builtin = 0",
                (id.to_string(),),
            )
            .map_err(db_error_in("deleting an equaliser preset"))?;

        if removed == 0 {
            return Err(CoreError::invalid(
                "eq preset",
                "there is no such preset of your own to delete",
            ));
        }
        Ok(())
    }
}

/// One row, before it is turned into a preset.
struct PresetRow {
    id: String,
    profile_id: Option<String>,
    name: String,
    is_builtin: bool,
    mode: String,
    bass: f64,
    mid: f64,
    treble: f64,
    bands: Option<String>,
    created_at: i64,
    updated_at: i64,
}

fn read(row: &Row<'_>) -> rusqlite::Result<PresetRow> {
    Ok(PresetRow {
        id: row.get(0)?,
        profile_id: row.get(1)?,
        name: row.get(2)?,
        is_builtin: row.get::<_, i64>(3)? != 0,
        mode: row.get(4)?,
        bass: row.get(5)?,
        mid: row.get(6)?,
        treble: row.get(7)?,
        bands: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn into_domain(row: PresetRow) -> Result<EqPreset> {
    let mode = EqMode::parse(&row.mode)?;
    let advanced = match row.bands.as_deref() {
        Some(json) => decode_bands(json)?,
        None => default_advanced_bands(),
    };
    if mode == EqMode::Advanced {
        validate_advanced_bands(&advanced)?;
    }

    Ok(EqPreset {
        id: EqPresetId::parse(&row.id)?,
        profile_id: row
            .profile_id
            .as_deref()
            .map(ProfileId::parse)
            .transpose()?,
        name: row.name,
        is_builtin: row.is_builtin,
        mode,
        simple: SimpleEq {
            bass: GainDb::new(row.bass as f32)?,
            mid: GainDb::new(row.mid as f32)?,
            treble: GainDb::new(row.treble as f32)?,
        },
        advanced,
        created_at: Timestamp::from_millis(row.created_at),
        updated_at: Timestamp::from_millis(row.updated_at),
    })
}

/// Writes the bands as an array of objects, with the field names spelled out.
///
/// Long names in a column nobody types by hand are still worth it: this is the
/// one part of the schema somebody might open in a text editor to see what a
/// preset is, and `q` next to `frequency_hz` says which number is which.
fn encode_bands(bands: &[EqBand]) -> String {
    let items: Vec<Value> = bands
        .iter()
        .map(|band| {
            serde_json::json!({
                "frequency_hz": band.frequency_hz(),
                "q": band.q(),
                "gain_db": band.gain().as_db(),
            })
        })
        .collect();
    Value::Array(items).to_string()
}

fn decode_bands(stored: &str) -> Result<Vec<EqBand>> {
    let json: Value = serde_json::from_str(stored)
        .map_err(|err| CoreError::invalid("eq bands", format!("stored badly: {err}")))?;

    let Value::Array(items) = json else {
        return Err(CoreError::invalid("eq bands", "expected a list of bands"));
    };

    items
        .iter()
        .map(|item| {
            let frequency_hz = number(item, "frequency_hz")? as u32;
            let q = number(item, "q")? as f32;
            let gain_db = number(item, "gain_db")? as f32;
            EqBand::new(frequency_hz, q, GainDb::new(gain_db)?)
        })
        .collect()
}

fn number(item: &Value, field: &str) -> Result<f64> {
    item.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| CoreError::invalid("eq bands", format!("a band has no {field}")))
}
