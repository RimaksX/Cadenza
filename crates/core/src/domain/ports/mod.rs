//! Ports: the interfaces infrastructure must implement.
//!
//! Every escape from pure computation — storage, audio devices, the filesystem,
//! the clock, OS scheduling — goes through a trait declared here. The domain
//! depends on these traits; `cadenza-infra` depends on the domain and provides
//! the implementations. That inversion is what keeps `core -> infra` off the
//! dependency graph (PROJECT_MASTER 4.4).
//!
//! # Why these are synchronous
//!
//! No async runtime is part of the stack: rusqlite, cpal and Symphonia are all
//! blocking APIs, and a desktop player has no thousands of concurrent
//! connections to justify one. Long-running work (scanning, hashing, feature
//! extraction) runs on ordinary background threads with their priority lowered
//! through [`system_priority::SystemPriorityPort`].
//!
//! Every port is `Send + Sync` so that those background threads can share them.

pub mod artwork_cache;
pub mod audio_engine;
pub mod clock;
pub mod decoder;
pub mod event_bus;
pub mod feature_extractor;
pub mod fetcher;
pub mod file_system;
pub mod file_watcher;
pub mod folder_picker;
pub mod log;
pub mod metadata_reader;
pub mod repositories;
pub mod system_priority;
