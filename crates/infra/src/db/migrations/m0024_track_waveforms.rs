//! The shape of each file's loudness, for its progress line.
//!
//! Global rather than per profile: it describes the file. A few hundred bytes
//! a row, one row per file that has been played, and it goes with the file.

pub const SQL: &str = r#"
CREATE TABLE track_waveforms (
    media_file_id TEXT NOT NULL PRIMARY KEY REFERENCES media_files (id) ON DELETE CASCADE,
    heights       BLOB NOT NULL
) STRICT;
"#;
