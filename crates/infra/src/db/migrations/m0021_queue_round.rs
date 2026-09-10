//! A fifth lane for the queue: the round now playing.
//!
//! `history` was doing two jobs with opposite lifetimes. It is the back-stack
//! the previous-track button walks, so it has to survive everything; and it was
//! also the record of what the current pass had already played, which shuffle
//! consults and which has to be cleared whenever a pass begins. Sharing one
//! list made a round that never ended: nothing cleared the library's, so once
//! every track had been heard, shuffle had nothing left unheard to choose and
//! stopped for good. Measured on the owner's own database — 137 rows of history
//! over a library of 41.
//!
//! The lane column carries a `CHECK` naming the lanes it allows, and SQLite
//! cannot alter one. So this is the rebuild SQLite's own documentation
//! prescribes: a new table with the wider check, the rows copied across, the
//! old one dropped, the new one renamed. Nothing references `queue_entries`, so
//! there are no children to orphan on the way through.
//!
//! **What an existing listener gets.** Their history comes across untouched and
//! their round is empty, which is exactly the recovery: a round with nothing in
//! it is a round with everything still to play, so shuffle starts working again
//! on the next press rather than needing anybody to clear anything by hand.

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
