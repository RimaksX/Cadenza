//! What analysis learned about a file.

use cadenza_core::Result;
use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::ports::repositories::TrackFeaturesRepositoryPort;
use cadenza_core::domain::track::TrackFeatures;
use cadenza_core::domain::value_objects::{Bpm, Mode, MusicalKey, Timestamp};
use rusqlite::Row;

use crate::db::SqlitePool;
use crate::db::error::db_error_in;

const COLUMNS: &str = "media_file_id, bpm, bpm_confidence, key, mode, energy, loudness, \
     spectral_centroid, spectral_rolloff, danceability, valence, tempo_stability, \
     dynamic_range, extractor_version, analyzed_at";

/// Reads and writes `track_features`.
pub struct SqliteTrackFeaturesRepository {
    pool: SqlitePool,
}

impl SqliteTrackFeaturesRepository {
    /// Binds the adapter to a pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl TrackFeaturesRepositoryPort for SqliteTrackFeaturesRepository {
    fn get(&self, media_file_id: MediaFileId) -> Result<Option<TrackFeatures>> {
        let connection = self.pool.get()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM track_features WHERE media_file_id = ?1"
            ))
            .map_err(db_error_in("reading track features"))?;

        let mut rows = statement
            .query_map([media_file_id.to_string()], FeaturesRow::read)
            .map_err(db_error_in("reading track features"))?;

        match rows.next() {
            None => Ok(None),
            Some(row) => {
                let row = row.map_err(db_error_in("reading track features"))?;
                row.into_domain().map(Some)
            }
        }
    }

    fn save(&self, features: &TrackFeatures) -> Result<()> {
        let connection = self.pool.get()?;
        connection
            .execute(
                "INSERT INTO track_features
                     (media_file_id, bpm, bpm_confidence, key, mode, energy, loudness,
                      spectral_centroid, spectral_rolloff, danceability, valence,
                      tempo_stability, dynamic_range, extractor_version, analyzed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 -- Replacing rather than refusing: a newer extractor's answer is
                 -- the answer, and the old one has nothing left to say.
                 ON CONFLICT (media_file_id) DO UPDATE SET
                     bpm               = excluded.bpm,
                     bpm_confidence    = excluded.bpm_confidence,
                     key               = excluded.key,
                     mode              = excluded.mode,
                     energy            = excluded.energy,
                     loudness          = excluded.loudness,
                     spectral_centroid = excluded.spectral_centroid,
                     spectral_rolloff  = excluded.spectral_rolloff,
                     danceability      = excluded.danceability,
                     valence           = excluded.valence,
                     tempo_stability   = excluded.tempo_stability,
                     dynamic_range     = excluded.dynamic_range,
                     extractor_version = excluded.extractor_version,
                     analyzed_at       = excluded.analyzed_at",
                rusqlite::params![
                    features.media_file_id.to_string(),
                    features.bpm.map(Bpm::as_f32),
                    features.bpm_confidence,
                    features.key.map(|key| i64::from(key.pitch_class())),
                    features.key.map(|key| key.mode().as_str()),
                    features.energy,
                    features.loudness,
                    features.spectral_centroid,
                    features.spectral_rolloff,
                    features.danceability,
                    features.valence,
                    features.tempo_stability,
                    features.dynamic_range,
                    features.extractor_version.as_str(),
                    features.analyzed_at.as_millis(),
                ],
            )
            .map_err(db_error_in("saving track features"))?;
        Ok(())
    }

    fn count_for_extractor(&self, extractor_version: &str) -> Result<u64> {
        let connection = self.pool.get()?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM track_features WHERE extractor_version = ?1",
                [extractor_version],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error_in("counting analysed files"))
            .map(|count| count.max(0) as u64)
    }
}

/// One row of `track_features`, in the column types SQLite hands back.
struct FeaturesRow {
    media_file_id: String,
    bpm: Option<f32>,
    bpm_confidence: f32,
    key: Option<i64>,
    mode: Option<String>,
    energy: f32,
    loudness: f32,
    spectral_centroid: f32,
    spectral_rolloff: f32,
    danceability: f32,
    valence: f32,
    tempo_stability: f32,
    dynamic_range: f32,
    extractor_version: String,
    analyzed_at: i64,
}

impl FeaturesRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            media_file_id: row.get(0)?,
            bpm: row.get(1)?,
            bpm_confidence: row.get(2)?,
            key: row.get(3)?,
            mode: row.get(4)?,
            energy: row.get(5)?,
            loudness: row.get(6)?,
            spectral_centroid: row.get(7)?,
            spectral_rolloff: row.get(8)?,
            danceability: row.get(9)?,
            valence: row.get(10)?,
            tempo_stability: row.get(11)?,
            dynamic_range: row.get(12)?,
            extractor_version: row.get(13)?,
            analyzed_at: row.get(14)?,
        })
    }

    fn into_domain(self) -> Result<TrackFeatures> {
        // The table forbids one without the other, so a row carrying half a key
        // has been edited by hand; it reads as no key rather than as an error,
        // because a missing key is a case every caller already handles.
        let key = match (self.key, self.mode.as_deref()) {
            (Some(pitch_class), Some(mode)) => Some(MusicalKey::new(
                u8::try_from(pitch_class).unwrap_or(u8::MAX),
                Mode::parse(mode)?,
            )?),
            _ => None,
        };

        Ok(TrackFeatures {
            media_file_id: MediaFileId::parse(&self.media_file_id)?,
            bpm: self.bpm.map(Bpm::new).transpose()?,
            bpm_confidence: self.bpm_confidence,
            key,
            energy: self.energy,
            loudness: self.loudness,
            spectral_centroid: self.spectral_centroid,
            spectral_rolloff: self.spectral_rolloff,
            danceability: self.danceability,
            valence: self.valence,
            tempo_stability: self.tempo_stability,
            dynamic_range: self.dynamic_range,
            extractor_version: self.extractor_version,
            analyzed_at: Timestamp::from_millis(self.analyzed_at),
        })
    }
}
