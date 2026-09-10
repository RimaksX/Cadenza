//! Content hashing for duplicate detection.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use cadenza_core::{CoreError, Result};

/// How much of the file is read at a time.
///
/// Large enough that the syscall overhead disappears, small enough that hashing
/// a 40 MB lossless file does not hold a megabyte-scale buffer per worker.
const CHUNK: usize = 64 * 1024;

/// Hashes a file's bytes.
///
/// blake3 rather than a SHA family function: a 5000-track library is tens of
/// gigabytes to read, and the background budget is about 20% of one CPU
/// . Collision resistance is far beyond what "are these the
/// same file" needs either way.
///
/// ponytail: hashes the whole file, so editing a tag changes the hash and the
/// same recording with different tags is not recognised as a duplicate. Hashing
/// only the audio stream would fix that, and needs the decoder to find
/// where the stream starts and ends. Until then the review queue catches what
/// this misses, which is the safe direction to be wrong in — a missed duplicate
/// is a second row, a false one is a decision the listener has to make.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|err| {
        CoreError::FileSystem(format!("could not open {}: {err}", path.display()))
    })?;

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; CHUNK];

    loop {
        let read = file.read(&mut buffer).map_err(|err| {
            CoreError::FileSystem(format!("could not read {}: {err}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::hash_file;

    fn write(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("cadenza-hash-{}-{name}", std::process::id()));
        std::fs::write(&path, contents).expect("writing a fixture");
        path
    }

    #[test]
    fn identical_contents_hash_identically() {
        let first = write("a", b"the same bytes");
        let second = write("b", b"the same bytes");

        assert_eq!(
            hash_file(&first).expect("hashing"),
            hash_file(&second).expect("hashing"),
            "duplicate detection depends on this"
        );

        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }

    #[test]
    fn one_changed_byte_changes_the_hash() {
        let first = write("c", b"the same bytes");
        let second = write("d", b"the same byteS");

        assert_ne!(
            hash_file(&first).expect("hashing"),
            hash_file(&second).expect("hashing")
        );

        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }

    #[test]
    fn a_file_larger_than_one_chunk_hashes_correctly() {
        // Exercises the read loop rather than a single pass.
        let big = write("e", &vec![7_u8; super::CHUNK * 2 + 13]);
        let same = write("f", &vec![7_u8; super::CHUNK * 2 + 13]);

        assert_eq!(
            hash_file(&big).expect("hashing"),
            hash_file(&same).expect("hashing")
        );

        let _ = std::fs::remove_file(big);
        let _ = std::fs::remove_file(same);
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_hashed_as_empty() {
        let missing = std::env::temp_dir().join("cadenza-hash-does-not-exist");
        assert!(hash_file(&missing).is_err());
    }
}
