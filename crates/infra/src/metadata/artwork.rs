//! Cover art on disk.

use std::fs;
use std::path::{Path, PathBuf};

use cadenza_core::domain::policies::artwork_policy::{IMAGE_EXTENSIONS, image_extension};
use cadenza_core::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};
use cadenza_core::{CoreError, Result};

// The bytes are stored exactly as they came, and the *name* carries the format
// they turned out to be: what finally draws a cover decides what it is looking
// at from the extension, so a picture stored under a name of our own invention
// is a picture nothing can open (MASTER_ISSUES 62).

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

    /// One file per key, and the key is in the name.
    ///
    /// A chosen cover is prefixed with the profile that chose it, so the two
    /// kinds cannot collide and a listener's choices can be found by eye in a
    /// directory listing — which is what somebody clearing space will do.
    fn stem_for(cover: CoverOf) -> String {
        match cover {
            CoverOf::Track(media_file_id) => format!("{media_file_id}"),
            CoverOf::ChosenTrack(profile_id, media_file_id) => {
                format!("chosen-{profile_id}-{media_file_id}")
            }
            CoverOf::Playlist(playlist_id) => format!("playlist-{playlist_id}"),
        }
    }

    fn file_for(&self, cover: CoverOf, extension: &str) -> PathBuf {
        self.directory
            .join(format!("{}.{extension}", Self::stem_for(cover)))
    }
}

impl ArtworkCachePort for FileArtworkCache {
    fn store(&self, cover: CoverOf, image: &[u8]) -> Result<()> {
        let Some(extension) = image_extension(image) else {
            return Err(CoreError::invalid(
                "cover",
                "these bytes are not a picture in any format this can draw",
            ));
        };

        // The same cover in a new format must not leave the old one behind for
        // `path_for` to find first.
        self.remove(cover)?;

        let path = self.file_for(cover, extension);
        fs::write(&path, image).map_err(|err| {
            CoreError::FileSystem(format!("could not write {}: {err}", path.display()))
        })
    }

    fn path_for(&self, cover: CoverOf) -> Option<PathBuf> {
        IMAGE_EXTENSIONS
            .iter()
            .map(|extension| self.file_for(cover, extension))
            .find(|path| path.is_file())
    }

    fn remove(&self, cover: CoverOf) -> Result<()> {
        // Every extension it could be under, and the one earlier versions wrote
        // before a name had to say what it held.
        for extension in IMAGE_EXTENSIONS.iter().chain(std::iter::once(&"img")) {
            let path = self.file_for(cover, extension);
            match fs::remove_file(&path) {
                Ok(()) => {}
                // Removing artwork that was never cached is what the caller
                // wanted.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(CoreError::FileSystem(format!(
                        "could not remove {}: {err}",
                        path.display()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The directory a cache would use under a given local data root.
#[must_use]
pub fn directory_under(local_root: &Path) -> PathBuf {
    local_root.join("cache").join("artwork")
}

#[cfg(test)]
mod tests {
    use super::FileArtworkCache;
    use cadenza_core::domain::ids::{MediaFileId, PlaylistId, ProfileId};
    use cadenza_core::domain::policies::artwork_policy::looks_like_an_image;
    use cadenza_core::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};

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

        assert!(
            cache.path_for(CoverOf::Track(id)).is_none(),
            "nothing is cached yet"
        );

        cache
            .store(CoverOf::Track(id), b"\xFF\xD8\xFFfake jpeg")
            .expect("storing");
        let path = cache
            .path_for(CoverOf::Track(id))
            .expect("it is cached now");
        assert_eq!(
            std::fs::read(&path).expect("reading"),
            b"\xFF\xD8\xFFfake jpeg"
        );

        cache.remove(CoverOf::Track(id)).expect("removing");
        assert!(cache.path_for(CoverOf::Track(id)).is_none());
        cache
            .remove(CoverOf::Track(id))
            .expect("removing twice is not an error");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn storing_again_replaces_the_previous_image() {
        let (root, cache) = cache("replace");
        let id = MediaFileId::new();

        cache
            .store(CoverOf::Track(id), b"\x89PNG\r\n\x1a\nfirst")
            .expect("storing");
        cache
            .store(CoverOf::Track(id), b"\x89PNG\r\n\x1a\nsecond")
            .expect("replacing");

        let path = cache.path_for(CoverOf::Track(id)).expect("cached");
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

    /// The three kinds of cover live in the same directory and must not be
    /// able to overwrite one another — the same recording can have a cover of
    /// its own and a different one chosen by each listener.
    #[test]
    fn the_three_keys_are_three_files() {
        let (root, cache) = cache("keys");
        let track = MediaFileId::new();
        let sasha = ProfileId::new();
        let list = PlaylistId::new();

        for (cover, bytes) in [
            (CoverOf::Track(track), &b"GIF89a from the file"[..]),
            (CoverOf::ChosenTrack(sasha, track), &b"GIF89a chosen"[..]),
            (CoverOf::Playlist(list), &b"GIF89a for the list"[..]),
        ] {
            cache.store(cover, bytes).expect("storing");
        }

        for (cover, expected) in [
            (CoverOf::Track(track), &b"from the file"[..]),
            (CoverOf::ChosenTrack(sasha, track), &b"chosen"[..]),
            (CoverOf::Playlist(list), &b"for the list"[..]),
        ] {
            let path = cache.path_for(cover).expect("each was stored");
            let read = std::fs::read(&path).expect("reading");
            assert!(read.ends_with(expected), "one key overwrote another");
        }

        // And a listener can take their choice back without touching what the
        // file itself carries.
        cache
            .remove(CoverOf::ChosenTrack(sasha, track))
            .expect("removing");
        assert!(cache.path_for(CoverOf::ChosenTrack(sasha, track)).is_none());
        assert!(cache.path_for(CoverOf::Track(track)).is_some());

        let _ = std::fs::remove_dir_all(&root);
    }
}
