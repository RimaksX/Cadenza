//! Listening events and their daily rollups.
//!
//! Everything here is written only while a profile has history enabled, and
//! purged after its retention window (PROJECT_MASTER 2.6).
//!
//! `play_events.radio_session_id` and the daily tables reference `radio_sessions`
//! and `mood_presets`, which migration 8 creates. SQLite resolves foreign keys
//! when rows are written, not when tables are declared, so a forward reference
//! is legal — and by the time anything is inserted, migration 8 has run.

pub const SQL: &str = r#"
CREATE TABLE play_events (
    id               TEXT    PRIMARY KEY,
    profile_id       TEXT    NOT NULL REFERENCES profiles (id)    ON DELETE CASCADE,
    media_file_id    TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,

    source           TEXT    NOT NULL
                     CHECK (source IN ('library', 'playlist', 'radio', 'manual')),

    radio_session_id TEXT    REFERENCES radio_sessions (id) ON DELETE SET NULL,

    started_at       INTEGER NOT NULL,
    ended_at         INTEGER CHECK (ended_at IS NULL OR ended_at >= started_at),

    -- Audio actually heard, excluding paused time.
    played_ms        INTEGER NOT NULL CHECK (played_ms >= 0),

    -- Copied from the file rather than joined: the file may later be removed or
    -- replaced, and a completion rate computed against a missing file is
    -- worthless.
    duration_ms      INTEGER NOT NULL CHECK (duration_ms >= 0),

    completed        INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
    skipped          INTEGER NOT NULL DEFAULT 0 CHECK (skipped   IN (0, 1)),

    -- The domain models the outcome as one enum precisely because these two
    -- flags can otherwise contradict each other. The database agrees.
    CHECK (NOT (completed = 1 AND skipped = 1)),

    -- Only a radio listen may name a radio session.
    CHECK (source = 'radio' OR radio_session_id IS NULL)
) STRICT;

-- The retention sweep and the "recent listens" query.
CREATE INDEX play_events_recent ON play_events (profile_id, started_at);
-- Play counts and skip rates per track.
CREATE INDEX play_events_track ON play_events (profile_id, media_file_id);

-- `date` stays TEXT here, unlike every instant in this schema: a civil date in
-- the listener's own timezone is a different thing from a point in time, and
-- 'YYYY-MM-DD' sorts and groups correctly as text.
CREATE TABLE daily_track_stats (
    profile_id     TEXT    NOT NULL REFERENCES profiles (id)    ON DELETE CASCADE,
    date           TEXT    NOT NULL CHECK (date LIKE '____-__-__'),
    media_file_id  TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,
    plays          INTEGER NOT NULL DEFAULT 0 CHECK (plays >= 0),
    seconds_played INTEGER NOT NULL DEFAULT 0 CHECK (seconds_played >= 0),
    skips          INTEGER NOT NULL DEFAULT 0 CHECK (skips >= 0),
    PRIMARY KEY (profile_id, date, media_file_id)
) STRICT;

CREATE TABLE daily_artist_stats (
    profile_id     TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    date           TEXT    NOT NULL CHECK (date LIKE '____-__-__'),
    artist_id      TEXT    NOT NULL REFERENCES artists (id)  ON DELETE CASCADE,
    plays          INTEGER NOT NULL DEFAULT 0 CHECK (plays >= 0),
    seconds_played INTEGER NOT NULL DEFAULT 0 CHECK (seconds_played >= 0),
    PRIMARY KEY (profile_id, date, artist_id)
) STRICT;

CREATE TABLE daily_genre_stats (
    profile_id     TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    date           TEXT    NOT NULL CHECK (date LIKE '____-__-__'),
    genre_id       TEXT    NOT NULL REFERENCES genres (id)   ON DELETE CASCADE,
    plays          INTEGER NOT NULL DEFAULT 0 CHECK (plays >= 0),
    seconds_played INTEGER NOT NULL DEFAULT 0 CHECK (seconds_played >= 0),
    PRIMARY KEY (profile_id, date, genre_id)
) STRICT;

CREATE TABLE daily_radio_stats (
    profile_id     TEXT    NOT NULL REFERENCES profiles (id)     ON DELETE CASCADE,
    date           TEXT    NOT NULL CHECK (date LIKE '____-__-__'),
    mood_id        TEXT    NOT NULL REFERENCES mood_presets (id) ON DELETE CASCADE,
    sessions       INTEGER NOT NULL DEFAULT 0 CHECK (sessions >= 0),
    seconds_played INTEGER NOT NULL DEFAULT 0 CHECK (seconds_played >= 0),
    skips          INTEGER NOT NULL DEFAULT 0 CHECK (skips >= 0),
    PRIMARY KEY (profile_id, date, mood_id)
) STRICT;
"#;
