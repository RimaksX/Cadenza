//! Which tracks each profile has, and how that profile labels them.
//!
//! A `metadata_override_json` column is absent: the typed columns beside it
//! express the same thing and can actually be queried and indexed.

pub const SQL: &str = r#"
CREATE TABLE profile_tracks (
    profile_id    TEXT    NOT NULL REFERENCES profiles (id)    ON DELETE CASCADE,
    media_file_id TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,

    -- The effective values: seeded from tags, overridden by the listener.
    -- Two profiles may legitimately disagree about the same file.
    title         TEXT    NOT NULL,
    artist_id     TEXT    REFERENCES artists (id) ON DELETE SET NULL,
    album_id      TEXT    REFERENCES albums (id)  ON DELETE SET NULL,
    track_no      INTEGER CHECK (track_no IS NULL OR track_no > 0),
    disc_no       INTEGER CHECK (disc_no  IS NULL OR disc_no  > 0),
    year          INTEGER,

    added_at      INTEGER NOT NULL,

    -- A tombstone, not a delete: the file stays on disk and in the catalogue,
 -- and other profiles keep their copy.
    removed_at    INTEGER CHECK (removed_at IS NULL OR removed_at >= added_at),

    PRIMARY KEY (profile_id, media_file_id)
) STRICT;

-- The library listing. Partial, because tombstones are never listed and there
-- can be many of them.
CREATE INDEX profile_tracks_live ON profile_tracks (profile_id) WHERE removed_at IS NULL;
CREATE INDEX profile_tracks_artist ON profile_tracks (profile_id, artist_id);
CREATE INDEX profile_tracks_album  ON profile_tracks (profile_id, album_id);

CREATE TABLE track_genres (
    media_file_id TEXT NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,
    genre_id      TEXT NOT NULL REFERENCES genres (id)      ON DELETE CASCADE,
    PRIMARY KEY (media_file_id, genre_id)
) STRICT;

CREATE INDEX track_genres_genre ON track_genres (genre_id);
"#;
