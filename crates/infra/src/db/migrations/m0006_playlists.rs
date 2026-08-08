//! Playlists and their entries.

pub const SQL: &str = r#"
CREATE TABLE playlists (
    id          TEXT    PRIMARY KEY,
    profile_id  TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    description TEXT,

    is_smart    INTEGER NOT NULL DEFAULT 0 CHECK (is_smart IN (0, 1)),
    -- The rule language is defined with smart playlists in M7; opaque until then.
    rule_json   TEXT,

    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,

    -- A smart playlist with no rule would silently be an empty one.
    CHECK (is_smart = 0 OR rule_json IS NOT NULL)
) STRICT;

CREATE INDEX playlists_profile ON playlists (profile_id);
CREATE UNIQUE INDEX playlists_name_key ON playlists (profile_id, name);

CREATE TABLE playlist_items (
    id            TEXT    PRIMARY KEY,
    playlist_id   TEXT    NOT NULL REFERENCES playlists (id)   ON DELETE CASCADE,
    media_file_id TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,
    position      INTEGER NOT NULL CHECK (position >= 0),
    added_at      INTEGER NOT NULL
) STRICT;

-- Deliberately not unique on (playlist_id, position): drag-and-drop reordering
-- renumbers many rows in one transaction and passes through states where two
-- rows briefly share a position. SQLite has no deferrable unique constraint, so
-- enforcing it here would mean rewriting the reorder as a slow dance around it.
CREATE INDEX playlist_items_order ON playlist_items (playlist_id, position);
"#;
