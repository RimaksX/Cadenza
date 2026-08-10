//! Shared test helpers: temp databases, fixtures, fake repositories, a controllable clock.
//!
//! Grows with the milestones that need it: `temp_db` and `test_clock` in M3,
//! `audio_fixtures` in M4, fake repositories when a test first needs one without
//! a database.

#![forbid(unsafe_code)]

pub mod audio_fixtures;
pub mod temp_db;
pub mod temp_dir;
pub mod test_clock;

pub use temp_db::TempDb;
pub use temp_dir::TempDir;
pub use test_clock::TestClock;
