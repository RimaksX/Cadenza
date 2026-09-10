//! Getting music off the disk and into the catalogue.
//!
//! Mechanism only. Walking directories, hashing bytes and watching for changes
//! live here; deciding whether a file is new, changed, a duplicate or a problem
//! is a business rule and lives in `cadenza-core`'s library service.

pub mod fetcher;
pub mod hash;
pub mod scanner;
pub mod watcher;

pub use fetcher::ExternalFetcher;
pub use scanner::LocalFileSystem;
pub use watcher::NotifyFileWatcher;
