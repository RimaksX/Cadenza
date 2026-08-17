//! Cadenza infrastructure: implements the ports declared in `cadenza-core`.
//!
//! This is the only layer allowed to touch SQLite, audio devices, the filesystem
//! and the operating system. It depends on `cadenza-core` and nothing else in the
//! workspace (PROJECT_MASTER 4.2).
//!
//! Populated by milestone: `db` in M2, `events` and `system` in M3, `metadata`
//! and `library` in M4, `audio` from M5, `analysis` in M11.

pub mod analysis;
pub mod audio;
pub mod db;
pub mod events;
pub mod library;
pub mod metadata;
pub mod system;
