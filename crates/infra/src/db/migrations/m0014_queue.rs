//! The saved playback queue.
//!
//! Section 7 of PROJECT_MASTER defines no table for it, while 2.3 requires
//! restoring the last queue and 2.5 makes it per-profile. This is the shape
//! that satisfies `QueueRepositoryPort` (MASTER_ISSUES 31).
//!
//! One table for the three lanes rather than three tables: they hold the same
//! thing in a different role, and the role is one column.

pub const SQL: &str = r#"
CREATE TABLE queue_state (
    profile_id  TEXT    PRIMARY KEY REFERENCES profiles (id) ON DELETE CASCADE,
    repeat_mode TEXT    NOT NULL CHECK (repeat_mode IN ('off', 'all', 'one')),
    shuffle     INTEGER NOT NULL CHECK (shuffle IN (0, 1)),
    updated_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE queue_entries (
    profile_id    TEXT    NOT NULL REFERENCES queue_state (profile_id) ON DELETE CASCADE,

    -- Which list the entry is in. 'current' holds at most one row, at position 0.
    lane          TEXT    NOT NULL CHECK (lane IN ('current', 'manual', 'upcoming', 'history')),
    position      INTEGER NOT NULL CHECK (position >= 0),

    -- A queued track whose file leaves the catalogue leaves the queue with it:
    -- there is nothing left to play.
    media_file_id TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,

    origin        TEXT    NOT NULL CHECK (origin IN ('library', 'playlist', 'radio')),
    origin_id     TEXT,

    PRIMARY KEY (profile_id, lane, position),

    -- Library entries come from nowhere in particular; the other two name the
    -- playlist or radio session they belong to, and a null there would lose it.
    CHECK ((origin = 'library') = (origin_id IS NULL))
) STRICT;
"#;
