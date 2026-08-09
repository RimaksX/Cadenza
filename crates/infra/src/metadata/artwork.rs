//! Cover art on disk.

use std::fs;
use std::path::{Path, PathBuf};

use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::ports::artwork_cache::ArtworkCachePort;
use cadenza_core::{CoreError, Result};

/// Extension every cached image is written with.
///
/// The bytes are stored exactly as they came out of the tag, whatever format
/// that is. The extension is a label for humans browsing the cache; the image
/// decoder in M6 sniffs the content rather than trusting it.
const CACHE_EXTENSION: &str = "img";

/// Stores cover images under `%LOCALAPPDATA%/Cadenza/cache/artwork`.
pub struct FileArtworkCache {
    directory: PathBuf,
}

impl FileArtworkCache {
    /// Binds the cache to a directory, creating it if needed.
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory).map_err(|err| {
            CoreError::FileSystem(format!(
                "could not create the artwork cache at {}: {err}",
                directory.display()
            ))
        })?;
        Ok(Self { directory })
    }

    fn file_for(&self, media_file_id: MediaFileId) -> PathBuf {
        self.directory
            .join(format!("{media_file_id}.{CACHE_EXTENSION}"))
    }
}

impl ArtworkCachePort for FileArtworkCache {
    fn store(&self, media_file_id: MediaFileId, image: &[u8]) -> Result<()> {
        let path = self.file_for(media_file_id);
        fs::write(&path, image).map_err(|err| {
            CoreError::FileSystem(format!("could not write {}: {err}", path.display()))
        })
    }

    fn path_for(&self, media_file_id: MediaFileId) -> Option<PathBuf> {
        let path = self.file_for(media_file_id);
        path.is_file().then_some(path)
    }

    fn remove(&self, media_file_id: MediaFileId) -> Result<()> {
        let path = self.file_for(media_file_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Removing artwork that was never cached is what the caller wanted.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(CoreError::FileSystem(format!(
                "could not remove {}: {err}",
                path.display()
            ))),
        }
    }
}

/// True when the bytes look like an image format worth caching.
///
/// Tags occasionally carry a placeholder, a text file, or nothing useful. A
/// four-byte check costs nothing and keeps rubbish out of the cache.
#[must_use]
pub fn looks_like_an_image(bytes: &[u8]) -> bool {
    const JPEG: [u8; 3] = [0xFF, 0xD8, 0xFF];
    const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const GIF: [u8; 3] = [b'G', b'I', b'F'];
    const BMP: [u8; 2] = [b'B', b'M'];

    bytes.starts_with(&JPEG)
        || bytes.starts_with(&PNG)
        || bytes.starts_with(&GIF)
        || bytes.starts_with(&BMP)
        // WEBP: "RIFF" .... "WEBP"
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
}

/// The directory a cache would use under a given local data root.
#[must_use]
pub fn directory_under(local_root: &Path) -> PathBuf {
    local_root.join("cache").join("artwork")
}

#[cfg(test)]
mod tests {
    use super::{FileArtworkCache, looks_like_an_image};
    use cadenza_core::domain::ids::MediaFileId;
    use cadenza_core::domain::ports::artwork_cache::ArtworkCachePort;

    fn cache(tag: &str) -> (std::path::PathBuf, FileArtworkCache) {
        let root =
            std::env::temp_dir().join(format!("cadenza-artwork-{}-{tag}", std::process::id()));
        let cache = FileArtworkCache::new(root.clone()).expect("a cache");
        (root, cache)
    }

    #[test]
    fn an_image_round_trips_and_can_be_removed() {
        let (root, cache) = cache("round-trip");
        let id = MediaFileId::new();

        assert!(cache.path_for(id).is_none(), "nothing is cached yet");

        cache.store(id, b"\xFF\xD8\xFFfake jpeg").expect("storing");
        let path = cache.path_for(id).expect("it is cached now");
        assert_eq!(
            std::fs::read(&path).expect("reading"),
            b"\xFF\xD8\xFFfake jpeg"
        );

        cache.remove(id).expect("removing");
        assert!(cache.path_for(id).is_none());
        cache.remove(id).expect("removing twice is not an error");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn storing_again_replaces_the_previous_image() {
        let (root, cache) = cache("replace");
        let id = MediaFileId::new();

        cache.store(id, b"\x89PNG\r\n\x1a\nfirst").expect("storing");
        cache
            .store(id, b"\x89PNG\r\n\x1a\nsecond")
            .expect("replacing");

        let path = cache.path_for(id).expect("cached");
        assert!(std::fs::read(&path).expect("reading").ends_with(b"second"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rubbish_is_not_mistaken_for_cover_art() {
        assert!(looks_like_an_image(b"\xFF\xD8\xFF..."));
        assert!(looks_like_an_image(b"\x89PNG\r\n\x1a\n..."));
        assert!(looks_like_an_image(b"RIFF\0\0\0\0WEBPmore"));

        assert!(!looks_like_an_image(b"this is a text file"));
        assert!(!looks_like_an_image(b""));
        assert!(!looks_like_an_image(b"\xFF\xD8"), "too short to be a JPEG");
    }
}
