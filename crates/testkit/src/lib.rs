//! Shared test helpers: temp databases, fixtures, fake repositories, a controllable clock.
//!
//! Grows with the milestones that need it: `temp_db` and `test_clock` in M3,
//! fake repositories when a test first needs one without a database, audio
//! fixtures from M5.

#![forbid(unsafe_code)]

pub mod temp_db;
pub mod test_clock;

pub use temp_db::TempDb;
pub use test_clock::TestClock;
