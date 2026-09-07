//! `Flat` is called `Standard`.
//!
//! The preset that leaves the sound alone was named for what it does to the
//! curve. "Flat" is the word an engineer reaches for and the word a listener
//! reads as *dull* — the one preset in the list nobody would try on purpose,
//! and it happens to be the one that means "as the record was made".
//!
//! By identifier rather than by name, because that is what a built-in preset
//! is: the same row on every machine, pointed at by whatever profile chose it.
//! Nothing else moves — not the setting a listener has, not which preset they
//! are on, not the curve, which was already the absence of one.
//!
//! Migration 15 is left exactly as it shipped. It has run on machines that are
//! not ours, and a migration that has run is a fact rather than a draft: the
//! only way to change what it did is another migration saying so.
//!
//! Safe against the unique index on `(IFNULL(profile_id, ''), name)`: a
//! listener's own preset called Standard carries their profile's identifier,
//! and this row carries none, so the two are different keys.

pub const SQL: &str = r#"
UPDATE eq_presets
    SET name = 'Standard'
    WHERE id = '00000000-0000-4000-8000-000000000001';
"#;
