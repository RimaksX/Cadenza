//! Library folders and key/value settings.

pub const SQL: &str = r#"
CREATE TABLE profile_folders (
    id                 TEXT    PRIMARY KEY,
    profile_id         TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    path               TEXT    NOT NULL,
    include_subfolders INTEGER NOT NULL DEFAULT 1 CHECK (include_subfolders IN (0, 1)),
    enabled            INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    last_scan_at       INTEGER
) STRICT;

-- Two profiles may watch the same folder independently; one profile may not
-- watch it twice.
CREATE UNIQUE INDEX profile_folders_path_key ON profile_folders (profile_id, path);

-- Global settings. Holds the active profile pointer, which is one of the few
-- pieces of state that belongs to the installation rather than to a listener.
CREATE TABLE app_settings (
    key        TEXT    PRIMARY KEY,
    value_json TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE profile_settings (
    profile_id TEXT    NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    key        TEXT    NOT NULL,
    value_json TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (profile_id, key)
) STRICT;
"#;
