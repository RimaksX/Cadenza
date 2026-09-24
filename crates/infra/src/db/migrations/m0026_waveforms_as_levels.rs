//! Waveforms kept as how far below its loudest each stretch sits, rather than
//! as bar heights: the drawing can then be tuned without measuring again. The
//! heights kept so far are cleared, and measured once more as levels.

pub const SQL: &str = r#"
DELETE FROM track_waveforms;
"#;
