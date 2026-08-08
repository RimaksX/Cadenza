//! Equaliser presets.

pub const SQL: &str = r#"
CREATE TABLE eq_presets (
    id                  TEXT    PRIMARY KEY,

    -- NULL for the nine built-in presets (PROJECT_MASTER 2.8).
    profile_id          TEXT    REFERENCES profiles (id) ON DELETE CASCADE,

    name                TEXT    NOT NULL,
    is_builtin          INTEGER NOT NULL DEFAULT 0 CHECK (is_builtin IN (0, 1)),

    mode                TEXT    NOT NULL DEFAULT 'simple'
                        CHECK (mode IN ('simple', 'advanced')),

    -- The same +/-12 dB range the domain's GainDb enforces. Wider ranges clip.
    simple_bass_gain    REAL    NOT NULL DEFAULT 0 CHECK (simple_bass_gain   BETWEEN -12 AND 12),
    simple_mid_gain     REAL    NOT NULL DEFAULT 0 CHECK (simple_mid_gain    BETWEEN -12 AND 12),
    simple_treble_gain  REAL    NOT NULL DEFAULT 0 CHECK (simple_treble_gain BETWEEN -12 AND 12),

    -- Ten bands. Validated against the expected centre frequencies on load,
    -- because gains mapped onto the wrong band would quietly wreck the sound.
    advanced_bands_json TEXT,

    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,

    CHECK ((is_builtin = 1) = (profile_id IS NULL)),
    CHECK (mode = 'simple' OR advanced_bands_json IS NOT NULL)
) STRICT;

CREATE UNIQUE INDEX eq_presets_name_key ON eq_presets (IFNULL(profile_id, ''), name);
"#;
