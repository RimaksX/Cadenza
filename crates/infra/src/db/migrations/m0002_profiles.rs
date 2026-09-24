//! Listener profiles.
//!
//! A `settings_json` column is deliberately absent: `profile_settings` is
//! already a key/value table, and two writable copies of one setting drift
//! apart.

pub const SQL: &str = r#"
CREATE TABLE profiles (
    id                     TEXT    PRIMARY KEY,
    name                   TEXT    NOT NULL,
    created_at             INTEGER NOT NULL,

    -- Off until the setup wizard asks. When off, nothing is recorded at all.
    history_enabled        INTEGER NOT NULL DEFAULT 0
                           CHECK (history_enabled IN (0, 1)),

    -- 30 days is a ceiling, not a default: a profile may keep less history,
    -- never more. Enforced here as well as in the domain so that a hand-edited
    -- row cannot quietly widen the window.
    history_retention_days INTEGER NOT NULL DEFAULT 30
                           CHECK (history_retention_days BETWEEN 1 AND 30),

    theme                  TEXT    NOT NULL DEFAULT 'dark'
                           CHECK (theme IN ('dark', 'light'))
) STRICT;

CREATE UNIQUE INDEX profiles_name_key ON profiles (name);
"#;
