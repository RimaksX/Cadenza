//! Use cases.
//!
//! A service loads entities through ports, applies the policies in
//! [`crate::domain::policies`], writes the result back and publishes an event.
//! It holds no rules of its own — anything that could be stated as "the rule is
//! X" belongs in a policy, where it can be tested without a database.
//!
//! Services arrive with the milestone that needs them: profiles in M3, the
//! library in M4, playback in M5, queue and playlists in M7.

pub mod library_service;
pub mod profile_service;

pub use library_service::{LibraryPorts, LibraryService, ScanReport};
pub use profile_service::ProfileService;
