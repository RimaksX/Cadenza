//! How loud a track is, how hard it pushes, and how much room it leaves.

use super::decode::Window;
use super::dsp::{percentile, rms, squash};
use super::spectral::Brightness;

/// How long a level measurement covers.
///
/// Fifty milliseconds is short enough to see a quiet passage between loud ones
/// and long enough not to be measuring individual drum hits.
const BLOCK_MS: u64 = 50;

/// The level a well-mastered track sits around, in dBFS.
///
/// The midpoint of the normalisation, so an ordinary record lands near the
/// middle of the range rather than at one end.
const TYPICAL_DBFS: f32 = -18.0;

/// How many decibels either side of typical fill the range.
const LEVEL_SPAN: f32 = 8.0;

/// The difference between loud and quiet passages, in decibels, that counts as
/// an ordinary amount of dynamic range.
const TYPICAL_RANGE_DB: f32 = 12.0;

/// Loudness and what it is made of, all normalised to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    /// Perceived intensity: how loud it is and how much of that is drive.
    pub energy: f32,
    /// Integrated level.
    pub loudness: f32,
    /// The distance between the quiet and loud parts.
    pub dynamic_range: f32,
}

impl Level {
    /// What silence measures.
    pub const SILENT: Self = Self {
        energy: 0.0,
        loudness: 0.0,
        dynamic_range: 0.0,
    };
}

/// Measures level over a window.
///
/// Brightness comes in rather than being measured again because energy is not
/// loudness: a bass drone at −12 dBFS is loud and inert, and a mix with the
/// same level and a top end is not. The two are weighted rather than one
/// standing for the other.
pub fn measure(window: &Window, brightness: &Brightness) -> Level {
    let block = (u64::from(window.rate) * BLOCK_MS / 1_000).max(1) as usize;
    if window.samples.len() < block {
        return Level::SILENT;
    }

    let mut levels: Vec<f32> = window
        .samples
        .chunks(block)
        .map(rms)
        .filter(|level| *level > 0.0)
        .map(decibels)
        .collect();

    if levels.is_empty() {
        return Level::SILENT;
    }

    // A mean over decibels rather than over amplitudes: the loud half of a
    // track would otherwise decide the answer on its own.
    let mean = levels.iter().sum::<f32>() / levels.len() as f32;
    let loud = percentile(&mut levels.clone(), 0.95);
    let quiet = percentile(&mut levels, 0.10);

    let loudness = squash(mean, TYPICAL_DBFS, LEVEL_SPAN);

    Level {
        // Two parts level, one part drive. Loud is most of what makes a track
        // feel energetic, but not all of it.
        energy: (loudness * 2.0 + brightness.drive) / 3.0,
        loudness,
        dynamic_range: squash(loud - quiet, TYPICAL_RANGE_DB, TYPICAL_RANGE_DB / 2.0),
    }
}

/// Amplitude as dBFS, with a floor so that silence is a number rather than an
/// infinity.
fn decibels(amplitude: f32) -> f32 {
    const FLOOR: f32 = -80.0;
    if amplitude <= 0.0 {
        return FLOOR;
    }
    (20.0 * amplitude.log10()).max(FLOOR)
}

#[cfg(test)]
mod tests {
    use super::{Level, decibels, measure};
    use crate::analysis::decode::Window;
    use crate::analysis::spectral::Brightness;

    fn steady(amplitude: f32) -> Window {
        let rate = 22_050;
        let samples = (0..rate)
            .map(|index| {
                (core::f32::consts::TAU * 220.0 * index as f32 / rate as f32).sin() * amplitude
            })
            .collect();
        Window { rate, samples }
    }

    fn flat_brightness(drive: f32) -> Brightness {
        Brightness {
            centroid: 0.5,
            rolloff: 0.5,
            drive,
        }
    }

    #[test]
    fn a_louder_track_measures_louder() {
        let quiet = measure(&steady(0.05), &flat_brightness(0.5));
        let loud = measure(&steady(0.8), &flat_brightness(0.5));

        assert!(loud.loudness > quiet.loudness);
        assert!(loud.energy > quiet.energy);
        assert!((0.0..=1.0).contains(&loud.loudness));
        assert!((0.0..=1.0).contains(&quiet.loudness));
    }

    #[test]
    fn drive_separates_energy_from_mere_volume() {
        let inert = measure(&steady(0.5), &flat_brightness(0.0));
        let driving = measure(&steady(0.5), &flat_brightness(1.0));

        assert!(driving.energy > inert.energy);
        assert_eq!(
            driving.loudness, inert.loudness,
            "the level itself is the same"
        );
    }

    #[test]
    fn a_steady_tone_has_almost_no_dynamic_range() {
        let level = measure(&steady(0.5), &flat_brightness(0.5));
        assert!(
            level.dynamic_range < 0.2,
            "nothing changes, so there is no range: {}",
            level.dynamic_range
        );
    }

    #[test]
    fn silence_measures_nothing_and_does_not_divide_by_it() {
        let silent = Window {
            rate: 22_050,
            samples: vec![0.0; 22_050],
        };
        assert_eq!(measure(&silent, &flat_brightness(0.0)), Level::SILENT);

        let too_short = Window {
            rate: 22_050,
            samples: vec![0.5; 10],
        };
        assert_eq!(measure(&too_short, &flat_brightness(0.0)), Level::SILENT);
    }

    #[test]
    fn decibels_have_a_floor() {
        assert_eq!(decibels(0.0), -80.0);
        assert!((decibels(1.0) - 0.0).abs() < 1e-6);
        assert!((decibels(0.5) + 6.02).abs() < 0.01);
    }
}
