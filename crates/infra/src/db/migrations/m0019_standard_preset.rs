//! `Flat` is called `Standard`.
//!
//! "Flat" is the word an engineer reaches for and the word a listener reads as
//! *dull* — the one preset nobody would try on purpose, and the one that means
//! "as the record was made".
//!
//! Renamed by identifier rather than by name, because that is what a built-in
//! preset is: the same row on every machine. Nothing else moves.
//!
//! **Migration 15 is left exactly as it shipped.** It has run on machines that
//! are not ours, and the only way to change what a migration did is another
//! migration saying so.
//!
//! Safe against the unique index on `(IFNULL(profile_id, ''), name)`: a
//! listener's own preset called Standard carries their profile's identifier and
//! this row carries none, so the two are different keys.

pub const SQL: &str = r#"
UPDATE eq_presets
    SET name = 'Standard'
    WHERE id = '00000000-0000-4000-8000-000000000001';
"#;
