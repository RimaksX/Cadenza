//! Walking the filesystem.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cadenza_core::domain::ports::file_system::{FileMetadata, FileSystemPort};
use cadenza_core::domain::value_objects::Timestamp;
use cadenza_core::{CoreError, Result};

use super::hash;

/// Reads the real filesystem.
///
/// Read-only by construction. Cadenza never deletes a file from disk — removing
/// a track removes it from the library only (PROJECT_MASTER 2.1) — so there is
/// no write path here to be called by mistake.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFileSystem;

impl FileSystemPort for LocalFileSystem {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let metadata = fs::metadata(path).map_err(|err| {
            CoreError::FileSystem(format!("could not stat {}: {err}", path.display()))
        })?;

        Ok(FileMetadata {
            size: metadata.len(),
            modified: modified_at(&metadata),
            is_dir: metadata.is_dir(),
        })
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(path).map_err(|err| {
            CoreError::FileSystem(format!("could not read {}: {err}", path.display()))
        })?;

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| {
                CoreError::FileSystem(format!(
                    "could not read an entry in {}: {err}",
                    path.display()
                ))
            })?;
            paths.push(entry.path());
        }

        // Sorted so a scan visits a folder in the same order every time, which
        // makes a scan report diffable and a failure reproducible.
        paths.sort();
        Ok(paths)
    }

    fn hash_file(&self, path: &Path) -> Result<String> {
        hash::hash_file(path)
    }
}

/// Reads the modification time, falling back to the epoch.
///
/// Some filesystems do not report one. The epoch is a safe fallback: it makes
/// the file look older than anything recorded, so the scanner treats it as
/// changed and re-reads it rather than trusting stale data.
fn modified_at(metadata: &fs::Metadata) -> Timestamp {
    let Ok(modified) = metadata.modified() else {
        return Timestamp::UNIX_EPOCH;
    };

    match modified.duration_since(UNIX_EPOCH) {
        Ok(since) => Timestamp::from_millis(i64::try_from(since.as_millis()).unwrap_or(i64::MAX)),
        Err(before) => Timestamp::from_millis(
            -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::LocalFileSystem;
    use cadenza_core::domain::ports::file_system::FileSystemPort;

    #[test]
    fn a_directory_is_listed_in_a_stable_order() {
        let root = std::env::temp_dir().join(format!("cadenza-scan-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).expect("a directory");
        for name in ["c.flac", "a.mp3", "b.wav"] {
            std::fs::write(root.join(name), b"x").expect("a file");
        }

        let files = LocalFileSystem;
        let listed = files.list_dir(&root).expect("listing");
        let names: Vec<String> = listed
            .iter()
            .map(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        assert_eq!(names, vec!["a.mp3", "b.wav", "c.flac", "sub"]);
        assert!(files.metadata(&root.join("sub")).expect("stat").is_dir);
        assert_eq!(files.metadata(&root.join("a.mp3")).expect("stat").size, 1);
        assert!(files.exists(&root.join("a.mp3")));
        assert!(!files.exists(&root.join("nothing.mp3")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_path_is_reported_rather_than_treated_as_empty() {
        let files = LocalFileSystem;
        let missing = std::env::temp_dir().join("cadenza-scan-does-not-exist");
        assert!(files.metadata(&missing).is_err());
        assert!(files.list_dir(&missing).is_err());
    }
}
