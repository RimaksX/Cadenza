//! Listener profiles.
//!
//! `settings_json` from PROJECT_MASTER 7.1 is deliberately absent: section 7.7
//! already defines a `profile_settings` key/value table, and two writable copies
//! of one setting drift apart. See `docs/MASTER_ISSUES.md`.

pub const SQL: &str = r#"
CREATE TABLE profiles (
    id                     TEXT    PRIMARY KEY,
    name                   TEXT    NOT NULL,
    created_at             INTEGER NOT NULL,

    -- Off until the setup wizard asks. When off, nothing is recorded at all
    -- (PROJECT_MASTER 1.4, 2.6).
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
