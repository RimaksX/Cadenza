//! Real audio files for tests to scan.
//!
//! WAV is generated rather than committed because it is the one required format
//! whose container is simple enough to write by hand: a 44-byte header and PCM
//! samples. MP3, FLAC and M4A need encoders, so fixtures in those formats have
//! to be real files — they arrived with the decoder, which is finally able
//! to verify them.
//!
//! Generating rather than committing also means a test can ask for two files
//! with deliberately identical or deliberately different contents, which is what
//! duplicate detection needs.

use std::path::{Path, PathBuf};

/// Sample rate of generated fixtures.
pub const SAMPLE_RATE: u32 = 44_100;

/// Channel count of generated fixtures.
pub const CHANNELS: u16 = 2;

/// Bits per sample of generated fixtures.
pub const BITS_PER_SAMPLE: u16 = 16;

/// Builds a valid WAV file in memory.
///
/// `fill` decides the sample values, so two calls with different fills produce
/// files of the same length with different contents — and therefore different
/// hashes. Two calls with the same fill produce byte-identical files, which is
/// exactly what a duplicate looks like.
#[must_use]
pub fn wav_bytes(seconds: u32, fill: i16) -> Vec<u8> {
    let bytes_per_sample = u32::from(BITS_PER_SAMPLE / 8);
    let block_align = u32::from(CHANNELS) * bytes_per_sample;
    let byte_rate = SAMPLE_RATE * block_align;
    let frames = SAMPLE_RATE * seconds;
    let data_len = frames * block_align;

    let mut wav = Vec::with_capacity(44 + data_len as usize);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes()); // chunk size for PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // format: uncompressed PCM
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&u16::try_from(block_align).unwrap_or(u16::MAX).to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames {
        for _ in 0..CHANNELS {
            wav.extend_from_slice(&fill.to_le_bytes());
        }
    }

    wav
}

/// Writes a WAV fixture to disk and returns its path.
///
/// Panics on failure: a fixture that cannot be written should stop the test
/// immediately rather than turn every call site into a `Result`.
pub fn write_wav(directory: &Path, name: &str, seconds: u32, fill: i16) -> PathBuf {
    std::fs::create_dir_all(directory)
        .unwrap_or_else(|err| panic!("could not create {}: {err}", directory.display()));

    let path = directory.join(name);
    std::fs::write(&path, wav_bytes(seconds, fill))
        .unwrap_or_else(|err| panic!("could not write {}: {err}", path.display()));
    path
}

#[cfg(test)]
mod tests {
    use super::{BITS_PER_SAMPLE, CHANNELS, SAMPLE_RATE, wav_bytes};

    #[test]
    fn the_header_says_what_the_data_is() {
        let wav = wav_bytes(1, 0);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");

        let expected = 44 + (SAMPLE_RATE as usize) * (CHANNELS as usize) * 2;
        assert_eq!(wav.len(), expected, "one second of 16-bit stereo");
        assert_eq!(BITS_PER_SAMPLE, 16);
    }

    #[test]
    fn the_same_fill_produces_the_same_bytes() {
        assert_eq!(wav_bytes(1, 42), wav_bytes(1, 42));
        assert_ne!(
            wav_bytes(1, 42),
            wav_bytes(1, 43),
            "duplicate detection depends on different fills differing"
        );
    }
}
