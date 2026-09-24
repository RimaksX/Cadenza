//! Ports: the interfaces infrastructure must implement.
//!
//! Every escape from pure computation — storage, audio devices, the filesystem,
//! the clock, OS scheduling — goes through a trait declared here. The domain
//! depends on these traits and infrastructure implements them, which is the
//! inversion that keeps `core -> infra` off the dependency graph.
//!
//! **Synchronous on purpose.** rusqlite, cpal and Symphonia are blocking APIs,
//! and a desktop player has no thousands of connections to justify an async
//! runtime. Long work runs on ordinary background threads with their priority
//! lowered through [`system_priority::SystemPriorityPort`], and every port is
//! `Send + Sync` so those threads can share them.

pub mod artwork_cache;
pub mod audio_engine;
pub mod backup;
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
pub mod saved_file;
pub mod system_priority;
pub mod waveform;
