//! Waveforms measured before their heights were drawn across each track's
//! own range. They were nearly all full height; clearing them has each one
//! measured again, the next time its track plays.

pub const SQL: &str = r#"
DELETE FROM track_waveforms;
"#;
