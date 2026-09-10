//! Mood and activity presets.

use cadenza_core::domain::ids::{MoodId, ProfileId};
use cadenza_core::domain::mood::{FeatureBand, MoodPreset, MoodRules};
use cadenza_core::domain::playback::TransitionProfile;
use cadenza_core::domain::ports::repositories::MoodRepositoryPort;
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::{CoreError, Result};
use rusqlite::Row;
use serde_json::Value;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "id, profile_id, name, is_builtin, feature_rules_json, \
     genre_boost_json, ranking_weights_json, transition_profile, created_at, updated_at";

/// Reads and writes `mood_presets`.
pub struct SqliteMoodRepository {
    pool: SqlitePool,
}

impl SqliteMoodRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl MoodRepositoryPort for SqliteMoodRepository {
    fn list_for_profile(&self, profile_id: ProfileId) -> Result<Vec<MoodPreset>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM mood_presets
                  WHERE profile_id IS NULL OR profile_id = ?1
                  -- Built-ins first, then the listener's own, each alphabetical.
                  ORDER BY is_builtin DESC, name"
            ))
            .map_err(db_error_in("listing moods"))?;

        let rows = statement
            .query_map((profile_id.to_string(),), MoodRow::read)
            .map_err(db_error_in("listing moods"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error_in("listing moods"))?;

        rows.into_iter().map(MoodRow::into_domain).collect()
    }

    fn get(&self, id: MoodId) -> Result<Option<MoodPreset>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!("SELECT {COLUMNS} FROM mood_presets WHERE id = ?1"))
            .map_err(db_error_in("reading a mood"))?;

        let mut rows = statement
            .query_map((id.to_string(),), MoodRow::read)
            .map_err(db_error_in("reading a mood"))?;

        match rows.next() {
            None => Ok(None),
            Some(row) => {
                let row = row.map_err(db_error_in("reading a mood"))?;
                row.into_domain().map(Some)
            }
        }
    }

    fn save(&self, preset: &MoodPreset) -> Result<()> {
        // A built-in is what shipped. Letting one be written would mean a
        // listener could edit their way out of a known-good starting point and
        // have no way back.
        if preset.is_builtin {
            return Err(CoreError::invalid(
                "mood preset",
                "built-in moods cannot be changed",
            ));
        }
        let profile_id = preset.profile_id.ok_or_else(|| {
            CoreError::invalid("mood preset", "a custom mood belongs to a profile")
        })?;

        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO mood_presets
                     (id, profile_id, name, is_builtin, feature_rules_json,
                      genre_boost_json, ranking_weights_json, transition_profile,
                      created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (id) DO UPDATE SET
                     name                 = excluded.name,
                     feature_rules_json   = excluded.feature_rules_json,
                     genre_boost_json     = excluded.genre_boost_json,
                     ranking_weights_json = excluded.ranking_weights_json,
                     transition_profile   = excluded.transition_profile,
                     updated_at           = excluded.updated_at",
                rusqlite::params![
                    preset.id.to_string(),
                    profile_id.to_string(),
                    preset.name.as_str(),
                    encode(&preset.rules),
                    preset.genre_boost_json.as_deref(),
                    preset.ranking_weights_json.as_deref(),
                    preset.transition.as_str(),
                    preset.created_at.as_millis(),
                    preset.updated_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving a mood"))?;
        Ok(())
    }

    fn delete(&self, id: MoodId) -> Result<()> {
        let connection = self.pool.get()?;
        let removed = connection
            .execute(
                "DELETE FROM mood_presets WHERE id = ?1 AND is_builtin = 0",
                (id.to_string(),),
            )
            .map_err(db_error_in("deleting a mood"))?;

        if removed == 0 {
            return Err(CoreError::invalid(
                "mood preset",
                "there is no such custom mood to delete",
            ));
        }
        Ok(())
    }
}

/// One row of `mood_presets`, in the column types SQLite hands back.
struct MoodRow {
    id: String,
    profile_id: Option<String>,
    name: String,
    is_builtin: i64,
    feature_rules_json: Option<String>,
    genre_boost_json: Option<String>,
    ranking_weights_json: Option<String>,
    transition_profile: String,
    created_at: i64,
    updated_at: i64,
}

impl MoodRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            profile_id: row.get(1)?,
            name: row.get(2)?,
            is_builtin: row.get(3)?,
            feature_rules_json: row.get(4)?,
            genre_boost_json: row.get(5)?,
            ranking_weights_json: row.get(6)?,
            transition_profile: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }

    fn into_domain(self) -> Result<MoodPreset> {
        Ok(MoodPreset {
            id: MoodId::parse(&self.id)?,
            profile_id: self
                .profile_id
                .as_deref()
                .map(ProfileId::parse)
                .transpose()?,
            name: self.name,
            is_builtin: self.is_builtin != 0,
            rules: decode(self.feature_rules_json.as_deref())?,
            genre_boost_json: self.genre_boost_json,
            ranking_weights_json: self.ranking_weights_json,
            transition: TransitionProfile::parse(&self.transition_profile)?,
            created_at: Timestamp::from_millis(self.created_at),
            updated_at: Timestamp::from_millis(self.updated_at),
        })
    }
}

/// Reads the stored rules.
///
/// Each band is `[low, high, falloff]`. A mood with no rules at all is a mood
/// that fits everything, so an absent column is not an error — and neither is
/// an unknown key, which is what lets a later version add a feature without
/// this one refusing to start.
fn decode(json: Option<&str>) -> Result<MoodRules> {
    let Some(json) = json else {
        return Ok(MoodRules::default());
    };

    let value: Value = serde_json::from_str(json)
        .map_err(|err| CoreError::invalid("mood rules", format!("{json:?} is not JSON: {err}")))?;

    let band = |name: &str| -> Result<Option<FeatureBand>> {
        let Some(entry) = value.get(name) else {
            return Ok(None);
        };
        let numbers: Vec<f32> = entry
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_f64().map(|number| number as f32))
                    .collect::<Vec<f32>>()
            })
            .unwrap_or_default();

        match numbers.as_slice() {
            [low, high, falloff] => Ok(Some(FeatureBand::new(*low, *high, *falloff))),
            _ => Err(CoreError::invalid(
                "mood rules",
                format!("{name} must be [low, high, falloff], got {entry}"),
            )),
        }
    };

    Ok(MoodRules {
        bpm: band("bpm")?,
        energy: band("energy")?,
        valence: band("valence")?,
        danceability: band("danceability")?,
    })
}

/// Writes rules back in the form [`decode`] reads.
fn encode(rules: &MoodRules) -> Option<String> {
    if rules.is_unconstrained() {
        return None;
    }

    let mut object = serde_json::Map::new();
    for (name, band) in [
        ("bpm", rules.bpm),
        ("energy", rules.energy),
        ("valence", rules.valence),
        ("danceability", rules.danceability),
    ] {
        if let Some(band) = band {
            object.insert(
                name.to_owned(),
                serde_json::json!([band.low, band.high, band.falloff]),
            );
        }
    }
    Some(Value::Object(object).to_string())
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use cadenza_core::domain::mood::{FeatureBand, MoodRules};

    #[test]
    fn rules_survive_the_round_trip() {
        let rules = MoodRules {
            bpm: Some(FeatureBand::new(125.0, 175.0, 25.0)),
            energy: Some(FeatureBand::new(0.65, 1.0, 0.25)),
            ..MoodRules::default()
        };

        let written = encode(&rules).expect("something to write");
        assert_eq!(decode(Some(&written)).expect("read back"), rules);
    }

    #[test]
    fn a_mood_that_asks_for_nothing_stores_nothing() {
        assert_eq!(encode(&MoodRules::default()), None);
        assert_eq!(decode(None).expect("read"), MoodRules::default());
    }

    #[test]
    fn a_band_that_is_not_three_numbers_is_refused() {
        assert!(decode(Some(r#"{"bpm":[120,160]}"#)).is_err());
        assert!(decode(Some("not json at all")).is_err());
    }

    #[test]
    fn a_feature_this_version_does_not_know_is_ignored() {
        // Forwards compatibility: a later Cadenza adding a band must not stop
        // this one from reading the row.
        let rules =
            decode(Some(r#"{"energy":[0.1,0.5,0.2],"loudness":[0.2,0.6,0.1]}"#)).expect("read");
        assert!(rules.energy.is_some());
        assert!(rules.bpm.is_none());
    }
}
