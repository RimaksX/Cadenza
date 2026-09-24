//! An album and an artist are places a queued track can come from, as a
//! playlist is. The origin's check names them; SQLite cannot change a check in
//! place, so the table is rebuilt the way migration 21 rebuilt it.

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

    origin        TEXT    NOT NULL
                  CHECK (origin IN ('library', 'playlist', 'radio', 'album', 'artist')),
    origin_id     TEXT,

    PRIMARY KEY (profile_id, lane, position),

    -- Library entries come from nowhere in particular; the others name the
    -- playlist, station, album or artist they belong to, and a null there
    -- would lose it.
    CHECK ((origin = 'library') = (origin_id IS NULL))
) STRICT;

INSERT INTO queue_entries_rebuilt
    (profile_id, lane, position, media_file_id, origin, origin_id)
SELECT profile_id, lane, position, media_file_id, origin, origin_id
FROM queue_entries;

DROP TABLE queue_entries;

ALTER TABLE queue_entries_rebuilt RENAME TO queue_entries;
"#;
