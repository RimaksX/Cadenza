//! How loud a track is, how hard it pushes, and how much room it leaves.

use super::activity::Activity;
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

/// What perceived level is worth in the energy blend.
///
/// Small, and it used to be two thirds. On a library of files mastered by
/// different people in different decades, integrated level is mostly a
/// measurement of the loudness war: the calmest track the owner has - bowed
/// strings and a choir, brickwalled to 3.7 dB of range - sat at -11 dBFS,
/// louder than the bass-led remix it was being ranked against
/// (`MASTER_ISSUES` 138). It still belongs in the answer, because a genuinely
/// quiet recording is genuinely less intense; it does not belong in charge.
const LEVEL_WEIGHT: f32 = 0.15;
/// What the top end is worth. Timbre, which is one of the five things Spotify
/// names behind its own energy.
const DRIVE_WEIGHT: f32 = 0.15;
/// What "how often something happens" is worth. Spotify's onset rate, and the
/// largest share, because it is what a listener means by an active track.
const ONSET_WEIGHT: f32 = 0.45;
/// What "how much events stand out" is worth. Spotify's general entropy.
const VARIATION_WEIGHT: f32 = 0.25;

/// Measures level over a window.
///
/// Brightness and activity come in rather than being measured again: energy is
/// not loudness, and the parts of it that are not loudness are measured
/// elsewhere. A bass drone at −12 dBFS is loud and inert; a mix at the same
/// level with a top end and a stick hitting something is not.
pub fn measure(window: &Window, brightness: &Brightness, activity: &Activity) -> Level {
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
        // Four parts, and level is the smallest of them. The weights sum to
        // one, so the answer stays inside the column whatever the parts do.
        energy: (ONSET_WEIGHT * activity.onsets
            + VARIATION_WEIGHT * activity.variation
            + LEVEL_WEIGHT * loudness
            + DRIVE_WEIGHT * brightness.drive)
            .clamp(0.0, 1.0),
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
    use crate::analysis::activity::Activity;
    use crate::analysis::decode::Window;
    use crate::analysis::spectral::Brightness;

    /// A track that is doing nothing in particular, so that a test about level
    /// is about level.
    fn still() -> Activity {
        Activity {
            onsets: 0.5,
            variation: 0.5,
        }
    }

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
        let quiet = measure(&steady(0.05), &flat_brightness(0.5), &still());
        let loud = measure(&steady(0.8), &flat_brightness(0.5), &still());

        assert!(loud.loudness > quiet.loudness);
        assert!(loud.energy > quiet.energy);
        assert!((0.0..=1.0).contains(&loud.loudness));
        assert!((0.0..=1.0).contains(&quiet.loudness));
    }

    #[test]
    fn what_is_happening_counts_for_more_than_what_it_measures_on_a_meter() {
        // The defect this is here for: a brickwalled calm recording measured
        // louder than a dynamic busy one, and energy was two thirds level, so
        // the calm one won (`MASTER_ISSUES` 138).
        let calm_but_loud = measure(
            &steady(0.9),
            &flat_brightness(0.5),
            &Activity {
                onsets: 0.05,
                variation: 0.1,
            },
        );
        let busy_but_quiet = measure(
            &steady(0.1),
            &flat_brightness(0.5),
            &Activity {
                onsets: 0.9,
                variation: 0.8,
            },
        );

        assert!(
            busy_but_quiet.energy > calm_but_loud.energy,
            "the busy one is the energetic one: {} vs {}",
            busy_but_quiet.energy,
            calm_but_loud.energy
        );
        assert!(
            calm_but_loud.loudness > busy_but_quiet.loudness,
            "even though the meter says otherwise"
        );
    }

    #[test]
    fn drive_separates_energy_from_mere_volume() {
        let inert = measure(&steady(0.5), &flat_brightness(0.0), &still());
        let driving = measure(&steady(0.5), &flat_brightness(1.0), &still());

        assert!(driving.energy > inert.energy);
        assert_eq!(
            driving.loudness, inert.loudness,
            "the level itself is the same"
        );
    }

    #[test]
    fn a_steady_tone_has_almost_no_dynamic_range() {
        let level = measure(&steady(0.5), &flat_brightness(0.5), &still());
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
        assert_eq!(
            measure(&silent, &flat_brightness(0.0), &still()),
            Level::SILENT
        );

        let too_short = Window {
            rate: 22_050,
            samples: vec![0.5; 10],
        };
        assert_eq!(
            measure(&too_short, &flat_brightness(0.0), &still()),
            Level::SILENT
        );
    }

    #[test]
    fn decibels_have_a_floor() {
        assert_eq!(decibels(0.0), -80.0);
        assert!((decibels(1.0) - 0.0).abs() < 1e-6);
        assert!((decibels(0.5) + 6.02).abs() < 0.01);
    }
}
