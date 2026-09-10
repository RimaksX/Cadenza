//! Files waiting for an import decision.
//!
//! Cadenza never silently discards a file or silently merges a duplicate.

pub const SQL: &str = r#"
CREATE TABLE import_review (
    id                      TEXT    PRIMARY KEY,

    -- An import decision is one listener's business alone.
    profile_id              TEXT    NOT NULL REFERENCES profiles (id)    ON DELETE CASCADE,
    media_file_id           TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,

    -- The file this one duplicates. Losing it must not lose the review entry.
    duplicate_media_file_id TEXT    REFERENCES media_files (id) ON DELETE SET NULL,

    reason                  TEXT    NOT NULL
                            CHECK (reason IN ('duplicate',
                                              'unreadable_metadata',
                                              'undecodable_audio',
                                              'missing_file')),

    state                   TEXT    NOT NULL DEFAULT 'pending'
                            CHECK (state IN ('pending', 'resolved', 'dismissed')),

    created_at              INTEGER NOT NULL,
    resolved_at             INTEGER,

    -- A duplicate entry that cannot say what it duplicates is unanswerable.
    CHECK (reason <> 'duplicate' OR duplicate_media_file_id IS NOT NULL),

    -- Resolved exactly when it has a resolution time.
    CHECK ((state = 'pending') = (resolved_at IS NULL))
) STRICT;

-- The badge count and the review screen both read only pending entries.
CREATE INDEX import_review_pending ON import_review (profile_id) WHERE state = 'pending';
"#;
