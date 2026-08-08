//! The domain layer: entities, value objects, policies and ports.
//!
//! Nothing here knows about SQLite, Slint, cpal or the operating system.

pub mod album;
pub mod analysis;
pub mod artist;
pub mod eq;
pub mod genre;
pub mod ids;
pub mod media_file;
pub mod mood;
pub mod playback;
pub mod playlist;
pub mod policies;
pub mod ports;
pub mod profile;
pub mod queue;
pub mod radio;
pub mod review;
pub mod settings;
pub mod stats;
pub mod track;
pub mod value_objects;
