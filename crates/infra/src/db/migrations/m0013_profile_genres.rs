//! Genres one profile disagrees about.
//!
//! `track_genres` describes the recording and is shared, which is right until a
//! listener corrects one: a profile may edit metadata locally, and that edit
//! must not reach anyone else. Title, artist, album and year
//! already have per-profile columns in `profile_tracks`; genre is many-to-many
//! and needs a table of its own.

pub const SQL: &str = r#"
-- Whether this profile has replaced the file's genres.
--
-- Separate from the rows below because "no genres at all" is a decision a
-- listener can make, and an empty override is otherwise indistinguishable from
-- having no override.
ALTER TABLE profile_tracks
    ADD COLUMN genres_overridden INTEGER NOT NULL DEFAULT 0
    CHECK (genres_overridden IN (0, 1));

CREATE TABLE profile_track_genres (
    profile_id    TEXT NOT NULL,
    media_file_id TEXT NOT NULL,
    genre_id      TEXT NOT NULL REFERENCES genres (id) ON DELETE CASCADE,

    PRIMARY KEY (profile_id, media_file_id, genre_id),

    -- Tied to the library entry rather than to the profile and the file
    -- separately: an override belongs to one listener's copy of one track, and
    -- deleting the profile takes it with them. Removing a track is a tombstone
    -- rather than a delete, so re-adding it later finds the override intact.
    FOREIGN KEY (profile_id, media_file_id)
        REFERENCES profile_tracks (profile_id, media_file_id) ON DELETE CASCADE
) STRICT;

-- Browsing a library by genre reads this before falling back to track_genres.
CREATE INDEX profile_track_genres_genre ON profile_track_genres (profile_id, genre_id);
"#;
