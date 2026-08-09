//! Reading tags and caching cover art.

pub mod artwork;
pub mod lofty_reader;
pub mod normalize;

pub use artwork::FileArtworkCache;
pub use lofty_reader::LoftyMetadataReader;
