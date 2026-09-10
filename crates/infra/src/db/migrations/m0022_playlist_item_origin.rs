//! Who put an entry in a playlist: a listener, or the play count.
//!
//! The favourites list holds both. Its played part is rebuilt whenever the
//! counts move, and rebuilding it must not disturb a track somebody pinned
//! there on purpose - so the two have to be told apart, and a row cannot say
//! which it is.
//!
//! An added column rather than a rebuild, because there is no `CHECK` to widen
//! this time. Every row that already exists was added by a listener, which is
//! what the default says: before this migration there was no other way for one
//! to get there.

pub const SQL: &str = r#"
ALTER TABLE playlist_items
    ADD COLUMN by_hand INTEGER NOT NULL DEFAULT 1 CHECK (by_hand IN (0, 1));
"#;
