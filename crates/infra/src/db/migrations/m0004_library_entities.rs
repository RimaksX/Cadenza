//! Artists, albums and genres.
//!
//! Global, like the files they describe.

pub const SQL: &str = r#"
CREATE TABLE artists (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    -- "Beatles, The". Derived on import, overridden by an explicit sort tag.
    sort_name  TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX artists_name_key ON artists (name);
CREATE INDEX artists_sort ON artists (sort_name);

CREATE TABLE albums (
    id         TEXT    PRIMARY KEY,
    -- NULL for compilations with no single credited artist. Losing the artist
    -- must not delete the album, hence SET NULL rather than CASCADE.
    artist_id  TEXT    REFERENCES artists (id) ON DELETE SET NULL,
    title      TEXT    NOT NULL,
    year       INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX albums_artist ON albums (artist_id);

-- Two different artists may each have an album called "Greatest Hits", so
-- uniqueness is on the pair. IFNULL keeps compilations from all colliding on
-- NULL, which never equals itself in a unique index.
CREATE UNIQUE INDEX albums_title_artist_key ON albums (title, IFNULL(artist_id, ''));

CREATE TABLE genres (
    id   TEXT PRIMARY KEY,
    -- Stored normalised (trimmed, lowercased) so tag spelling variants collapse.
    name TEXT NOT NULL UNIQUE
) STRICT;
"#;
