//! Physical files and the audio features derived from them.
//!
//! Both tables are global. They describe the music, not the listener, so two
//! profiles holding the same file share one row and one analysis.
//!
//! A `track_features.scale` column is absent: it and `mode` would name the same
//! major/minor property.

pub const SQL: &str = r#"
CREATE TABLE media_files (
    id                    TEXT    PRIMARY KEY,
    path                  TEXT    NOT NULL UNIQUE,

    -- Absent until the hashing job runs. Duplicate detection waits for it
    -- rather than guessing from the filename.
    file_hash             TEXT,

    file_size             INTEGER NOT NULL,
    file_mtime            INTEGER NOT NULL,

    format                TEXT    NOT NULL
                          CHECK (format IN ('mp3', 'aac', 'alac', 'flac', 'wav')),

    duration_ms           INTEGER NOT NULL CHECK (duration_ms >= 0),
    sample_rate           INTEGER NOT NULL CHECK (sample_rate > 0),
    channels              INTEGER NOT NULL CHECK (channels > 0),
    bitrate               INTEGER,

    metadata_version      TEXT,
    metadata_extracted_at INTEGER,

    file_state            TEXT    NOT NULL DEFAULT 'ok'
                          CHECK (file_state IN ('ok', 'missing', 'error')),

    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
) STRICT;

-- The duplicate-detection lookup. Partial, because most rows have no hash yet
-- and indexing thousands of NULLs helps nobody.
CREATE INDEX media_files_hash ON media_files (file_hash) WHERE file_hash IS NOT NULL;

-- Shuffle and radio exclude anything not playable, so they filter on this.
CREATE INDEX media_files_state ON media_files (file_state);

CREATE TABLE track_features (
    media_file_id     TEXT    PRIMARY KEY
                      REFERENCES media_files (id) ON DELETE CASCADE,

    -- NULL means detection failed or has not run. Callers must treat that as
    -- unknown; substituting zero would read as "no tempo at all".
    bpm               REAL    CHECK (bpm IS NULL OR bpm BETWEEN 20 AND 300),
    bpm_confidence    REAL    NOT NULL DEFAULT 0
                      CHECK (bpm_confidence BETWEEN 0 AND 1),

    key               INTEGER CHECK (key IS NULL OR key BETWEEN 0 AND 11),
    mode              TEXT    CHECK (mode IS NULL OR mode IN ('major', 'minor')),

    -- Every remaining feature is normalised to 0.0..=1.0.
    energy            REAL    NOT NULL CHECK (energy BETWEEN 0 AND 1),
    loudness          REAL    NOT NULL CHECK (loudness BETWEEN 0 AND 1),
    spectral_centroid REAL    NOT NULL CHECK (spectral_centroid BETWEEN 0 AND 1),
    spectral_rolloff  REAL    NOT NULL CHECK (spectral_rolloff BETWEEN 0 AND 1),
    danceability      REAL    NOT NULL CHECK (danceability BETWEEN 0 AND 1),
    valence           REAL    NOT NULL CHECK (valence BETWEEN 0 AND 1),
    tempo_stability   REAL    NOT NULL CHECK (tempo_stability BETWEEN 0 AND 1),
    dynamic_range     REAL    NOT NULL CHECK (dynamic_range BETWEEN 0 AND 1),

    -- Re-analysis happens only when this differs from the current extractor.
    extractor_version TEXT    NOT NULL,
    analyzed_at       INTEGER NOT NULL
) STRICT;

-- Finding what still needs re-analysing after an extractor upgrade.
CREATE INDEX track_features_extractor ON track_features (extractor_version);
"#;
