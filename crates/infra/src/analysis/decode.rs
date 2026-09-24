//! Reading the part of a file that gets analysed.
//!
//! Not the whole file. A library of five thousand tracks is fourteen hours of
//! audio per hour of music, and decoding all of it to learn a tempo is work
//! nobody hears the benefit of. What is taken instead is a window from the
//! middle, where a track is most itself: intros fade in, endings fade out, and
//! neither describes the song.

use std::path::Path;

use cadenza_core::Result;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition};

use crate::audio::symphonia_decoder::TrackStream;

/// How much audio is analysed.
///
/// A calibration knob. Ninety seconds is long enough for a tempo estimate to
/// settle over dozens of bars and for the key to survive a passing modulation,
/// and short enough that a whole library is minutes of work rather than hours.
pub const WINDOW: DurationMs = DurationMs::from_secs(90);

/// The rate everything downstream works at.
///
/// Half of CD rate, which keeps everything up to eleven kilohertz — cymbals
/// included, which is what "brightness" is mostly made of — and halves the cost
/// of every transform. Decoding costs the same either way; this is the part
/// that can be made cheaper without losing anything worth measuring.
pub const TARGET_RATE: u32 = 22_050;

/// A stretch of mono audio at a known rate.
#[derive(Debug, Clone)]
pub struct Window {
    /// Samples per second. Close to [`TARGET_RATE`], exactly a whole division
    /// of the file's own rate.
    pub rate: u32,
    /// Mono samples, `-1.0..=1.0`.
    pub samples: Vec<f32>,
}

impl Window {
    /// How long the window turned out to be.
    pub fn duration(&self) -> DurationMs {
        if self.rate == 0 {
            return DurationMs::ZERO;
        }
        DurationMs::from_millis(self.samples.len() as u64 * 1_000 / u64::from(self.rate))
    }
}

/// Decodes the middle of a file, mixed to mono and reduced to [`TARGET_RATE`].
///
/// A short file is read from where it starts: half of thirty seconds is still
/// the middle of it, and seeking into a file shorter than the window would
/// leave nothing behind the seek point.
pub fn read_window(path: &Path) -> Result<Window> {
    let mut stream = TrackStream::open(path)?;
    let info = stream.info();

    let channels = usize::from(info.channels).max(1);
    let step = (info.sample_rate / TARGET_RATE).max(1) as usize;
    let rate = info.sample_rate / step as u32;

    if info.duration > WINDOW {
        let middle = (info.duration.as_millis() - WINDOW.as_millis()) / 2;
        // A container that will not seek is read from the beginning instead.
        // Ninety seconds of the start is a worse answer than ninety from the
        // middle, and a better one than none.
        let _ = stream.seek(PlaybackPosition::from_millis(middle));
    }

    let wanted = (u64::from(rate) * WINDOW.as_millis() / 1_000) as usize;
    let mut samples = Vec::with_capacity(wanted);

    // Whole frames left over between packets: a packet boundary rarely falls on
    // a multiple of the decimation step, and dropping the remainder would put a
    // sub-sample hole in the signal every few milliseconds.
    let mut carry = 0.0f32;
    let mut carried = 0usize;

    while samples.len() < wanted {
        let Some(frames) = stream.next_frames()? else {
            break;
        };

        for frame in frames.chunks_exact(channels) {
            // Mixed down rather than taking one channel: a track with the beat
            // panned to one side would otherwise be analysed without it.
            let mono = frame.iter().sum::<f32>() / channels as f32;

            carry += mono;
            carried += 1;
            if carried == step {
                samples.push(carry / step as f32);
                carry = 0.0;
                carried = 0;

                if samples.len() == wanted {
                    break;
                }
            }
        }
    }

    Ok(Window { rate, samples })
}

#[cfg(test)]
mod tests {
    use super::{TARGET_RATE, WINDOW, Window, read_window};
    use cadenza_core::domain::value_objects::DurationMs;
    use cadenza_testkit::TempDir;
    use cadenza_testkit::audio_fixtures::write_wav;

    #[test]
    fn a_window_knows_how_long_it_is() {
        let window = Window {
            rate: 1_000,
            samples: vec![0.0; 2_500],
        };
        assert_eq!(window.duration(), DurationMs::from_millis(2_500));

        let broken = Window {
            rate: 0,
            samples: vec![0.0; 10],
        };
        assert_eq!(broken.duration(), DurationMs::ZERO);
    }

    #[test]
    fn a_long_file_gives_exactly_the_window_and_takes_it_from_the_middle() {
        let directory = TempDir::new("analysis-window");
        // Two minutes, so that ninety seconds cannot be reached without seeking
        // past the start and cannot overrun the end.
        let path = write_wav(directory.path(), "long.wav", 120, 6_000);

        let window = read_window(&path).expect("decoded");

        assert_eq!(
            window.samples.len(),
            (TARGET_RATE as u64 * WINDOW.as_millis() / 1_000) as usize,
            "the whole window was read, not what fitted before the file ended"
        );
        assert_eq!(window.duration(), WINDOW);
    }

    #[test]
    fn a_short_file_is_read_from_the_beginning_and_gives_what_it_has() {
        let directory = TempDir::new("analysis-decode");
        let path = write_wav(directory.path(), "two.wav", 2, 8_000);

        let window = read_window(&path).expect("decoded");

        assert_eq!(window.rate, TARGET_RATE, "44.1 kHz halves exactly");
        assert!(
            window.duration() >= DurationMs::from_millis(1_900)
                && window.duration() <= DurationMs::from_secs(2),
            "two seconds of file is two seconds of window, not ninety: {:?}",
            window.duration()
        );
        assert!(window.duration() < WINDOW);
        assert!(
            window.samples.iter().all(|sample| sample.abs() <= 1.0),
            "samples stay in range"
        );
    }
}
