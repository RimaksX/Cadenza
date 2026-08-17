//! Use cases.
//!
//! A service loads entities through ports, applies the policies in
//! [`crate::domain::policies`], writes the result back and publishes an event.
//! It holds no rules of its own — anything that could be stated as "the rule is
//! X" belongs in a policy, where it can be tested without a database.
//!
//! Services arrive with the milestone that needs them: profiles in M3, the
//! library in M4, playback in M5, queue and playlists in M7, the
//! equaliser in M9.

pub mod analysis_service;
pub mod eq_service;
pub mod library_service;
pub mod playback_service;
pub mod playlist_service;
pub mod profile_service;
pub mod queue_service;
pub mod radio_service;

pub use analysis_service::{AnalysisPorts, AnalysisProgress, AnalysisService};
pub use eq_service::{EqPorts, EqService};
pub use library_service::{LibraryPorts, LibraryService, ScanReport};
pub use playback_service::{PlaybackPorts, PlaybackService};
pub use playlist_service::{PlaylistPorts, PlaylistService, PlaylistSummary};
pub use profile_service::ProfileService;
pub use queue_service::{QueuePorts, QueueService};
pub use radio_service::{RadioPorts, RadioService};
