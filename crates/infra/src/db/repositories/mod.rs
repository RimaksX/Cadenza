//! SQLite implementations of the repository ports.
//!
//! Each adapter reads rows into a private `*Row` struct of plain column types
//! first, then converts that into a domain entity. Two steps rather than one
//! because the conversion can fail — a hand-edited theme, an out-of-range
//! retention window — and `rusqlite`'s row closure has no room for a domain
//! error. Splitting them also keeps the SQL and the validation legible
//! separately.
//!
//! Adapters arrive with the milestone that needs them: profiles and settings in
//! M3, the catalogue and the library in M4, playlists and the queue in M7.

pub mod catalog_repo;
pub mod media_file_repo;
pub mod profile_repo;
pub mod queue_repo;
pub mod review_repo;
pub mod settings_repo;
pub mod track_repo;

pub use catalog_repo::{SqliteAlbumRepository, SqliteArtistRepository, SqliteGenreRepository};
pub use media_file_repo::SqliteMediaFileRepository;
pub use profile_repo::SqliteProfileRepository;
pub use queue_repo::SqliteQueueRepository;
pub use review_repo::SqliteImportReviewRepository;
pub use settings_repo::SqliteSettingsRepository;
pub use track_repo::SqliteTrackRepository;
