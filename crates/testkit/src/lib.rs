//! Shared test helpers: temp databases, fixtures, fake repositories, a controllable clock.
//!
//! Grows with the milestones that need it: `temp_db` in M2, a test clock and fake
//! repositories in M3, audio fixtures from M5.

#![forbid(unsafe_code)]

pub mod temp_db;

pub use temp_db::TempDb;
