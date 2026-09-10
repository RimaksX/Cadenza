//! Cadenza infrastructure: implements the ports declared in `cadenza-core`.
//!
//! This is the only layer allowed to touch SQLite, audio devices, the filesystem
//! and the operating system. It depends on `cadenza-core` and nothing else in the
//! workspace.

pub mod analysis;
pub mod audio;
pub mod db;
pub mod events;
pub mod library;
pub mod metadata;
pub mod system;
