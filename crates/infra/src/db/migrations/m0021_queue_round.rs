//! A fifth lane for the queue: the round now playing.
//!
//! `history` was doing two jobs with opposite lifetimes — the back-stack the
//! previous button walks, which must survive everything, and the record of what
//! this pass has played, which must be cleared when a pass begins. Sharing one
//! list made a round that never ended: once every track had been heard, shuffle
//! had nothing unheard left and stopped for good. The owner's database had 137
//! rows of history over a library of 41.
//!
//! **The lane column carries a `CHECK`, and SQLite cannot alter one**, so this
//! is the table rebuild SQLite's own documentation prescribes. Nothing
//! references `queue_entries`, so no children are orphaned on the way through.
//!
//! An existing listener keeps their history and gets an empty round, which is
//! the recovery itself: an empty round is a round with everything still to
//! play, so shuffle works again on the next press with nobody clearing anything.

pub const SQL: &str = r#"
CREATE TABLE queue_entries_rebuilt (
    profile_id    TEXT    NOT NULL REFERENCES queue_state (profile_id) ON DELETE CASCADE,

    -- Which list the entry is in. 'current' holds at most one row, at position 0.
    -- 'history' is where the previous-track button walks; 'round' is what the
    -- pass now playing has already been through.
    lane          TEXT    NOT NULL
                  CHECK (lane IN ('current', 'manual', 'upcoming', 'history', 'round')),
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

INSERT INTO queue_entries_rebuilt
    (profile_id, lane, position, media_file_id, origin, origin_id)
SELECT profile_id, lane, position, media_file_id, origin, origin_id
FROM queue_entries;

DROP TABLE queue_entries;

ALTER TABLE queue_entries_rebuilt RENAME TO queue_entries;
"#;
