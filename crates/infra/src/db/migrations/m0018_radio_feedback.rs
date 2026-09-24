//! What the listener thought of a radio pick.
//!
//! Skip, like and dislike have to affect future generation, which means the
//! verdict has to outlive the moment it was given. It goes on the pick rather
//! than in a table of its own: a verdict is about *this track in this session*,
//! and the same track offered again next week in a different mood deserves to
//! be judged again rather than inheriting an old answer. Aggregating across
//! sessions is then a query, which is the shape that lets the weight of an old
//! opinion be reconsidered without a migration.
//!
//! `NULL` is the honest default and means "not judged", which is not the same
//! as neutral: a track nobody skipped is not thereby approved of.

pub const SQL: &str = r#"
ALTER TABLE radio_session_items
    ADD COLUMN feedback TEXT
    CHECK (feedback IS NULL OR feedback IN ('like', 'dislike', 'skip'));

-- What preference aggregation reads: every judged pick for one file.
CREATE INDEX radio_session_items_feedback
    ON radio_session_items (media_file_id)
    WHERE feedback IS NOT NULL;
"#;
