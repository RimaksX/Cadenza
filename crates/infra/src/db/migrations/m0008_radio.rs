//! Moods, radio sessions and the picks they made.

pub const SQL: &str = r#"
CREATE TABLE mood_presets (
    id                   TEXT    PRIMARY KEY,

    -- NULL for the built-in moods everyone shares. Built-ins are not user data,
    -- which is why they may omit the profile without breaking isolation.
    profile_id           TEXT    REFERENCES profiles (id) ON DELETE CASCADE,

    name                 TEXT    NOT NULL,
    is_builtin           INTEGER NOT NULL DEFAULT 0 CHECK (is_builtin IN (0, 1)),

    -- The shapes belong to the ranking formula that reads them.
    feature_rules_json   TEXT,
    genre_boost_json     TEXT,
    ranking_weights_json TEXT,

    transition_profile   TEXT    NOT NULL DEFAULT 'gapless'
                         CHECK (transition_profile IN ('gapless', 'crossfade')),

    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,

    -- Built-in exactly when there is no owning profile. Stops a custom mood from
    -- claiming to be built-in and becoming uneditable.
    CHECK ((is_builtin = 1) = (profile_id IS NULL))
) STRICT;

CREATE UNIQUE INDEX mood_presets_name_key ON mood_presets (IFNULL(profile_id, ''), name);

CREATE TABLE radio_sessions (
    id                 TEXT    PRIMARY KEY,
    profile_id         TEXT    NOT NULL REFERENCES profiles (id)     ON DELETE CASCADE,
    mood_id            TEXT    NOT NULL REFERENCES mood_presets (id) ON DELETE CASCADE,

    -- Present when the listener started radio from a specific track.
    seed_media_file_id TEXT    REFERENCES media_files (id) ON DELETE SET NULL,

    params_json        TEXT,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
) STRICT;

CREATE INDEX radio_sessions_recent ON radio_sessions (profile_id, created_at);

CREATE TABLE radio_session_items (
    id            TEXT    PRIMARY KEY,
    session_id    TEXT    NOT NULL REFERENCES radio_sessions (id) ON DELETE CASCADE,
    media_file_id TEXT    NOT NULL REFERENCES media_files (id)    ON DELETE CASCADE,
    position      INTEGER NOT NULL CHECK (position >= 0),

    -- Why this track was chosen. Kept because selection is a weighted formula
    -- rather than a model, so every pick can be explained.
    reason_json   TEXT,

    created_at    INTEGER NOT NULL
) STRICT;

CREATE INDEX radio_session_items_order ON radio_session_items (session_id, position);
"#;
