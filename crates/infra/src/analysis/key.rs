//! Which key a track is in, by how well its notes match each one.
//!
//! Chroma folded from the spectrum, correlated against the Krumhansl-Schmuckler
//! profiles — two vectors of twelve numbers, published in 1990, measured by
//! asking people how well a note fitted a key. A table, not a model.

use cadenza_core::domain::value_objects::{Mode, MusicalKey};

use super::dsp::{Spectra, squash};

/// The lowest frequency folded into chroma.
///
/// Below this the bins are too wide to name a note: at 22 050 Hz the transform
/// resolves about eleven hertz, which is a whole tone at the bottom of the bass
/// register and a rounding error at the top of it.
const LOWEST_HZ: f32 = 110.0;

/// The highest. Above it there is mostly percussion, which belongs to no key.
const HIGHEST_HZ: f32 = 3_520.0;

/// Concert pitch, from which every other note is counted.
const A4_HZ: f32 = 440.0;

/// The pitch class of A, since chroma is counted from C.
const A_PITCH_CLASS: usize = 9;

/// How well each note fits a major key (Krumhansl-Schmuckler).
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// And a minor one.
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// What the key estimate came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyEstimate {
    /// The key, absent when nothing fitted better than anything else.
    pub key: Option<MusicalKey>,
    /// How far ahead of the runner-up the winner was, `0.0..=1.0`.
    pub strength: f32,
}

impl KeyEstimate {
    /// What atonal or silent material measures.
    pub const NONE: Self = Self {
        key: None,
        strength: 0.0,
    };
}

/// Estimates the key of a window.
pub fn measure(spectra: &Spectra) -> KeyEstimate {
    let chroma = chroma(spectra);
    let total: f32 = chroma.iter().sum();
    if total <= f32::EPSILON {
        return KeyEstimate::NONE;
    }

    let mut best = (0.0f32, 0usize, Mode::Major);
    let mut runner_up = f32::NEG_INFINITY;

    for tonic in 0..12 {
        for (profile, mode) in [(MAJOR_PROFILE, Mode::Major), (MINOR_PROFILE, Mode::Minor)] {
            let score = fit(&chroma, &profile, tonic);
            if score > best.0 {
                runner_up = best.0;
                best = (score, tonic, mode);
            } else if score > runner_up {
                runner_up = score;
            }
        }
    }

    let Ok(key) = MusicalKey::new(best.1 as u8, best.2) else {
        return KeyEstimate::NONE;
    };

    // How much better the winner was than the next candidate. Two keys sharing
    // most of their notes score alike, so a small margin is the honest reading
    // of "it is one of these two" rather than a reason to refuse an answer.
    let margin = if best.0 > 0.0 {
        (best.0 - runner_up.max(0.0)) / best.0
    } else {
        0.0
    };

    KeyEstimate {
        key: Some(key),
        strength: squash(margin, 0.08, 0.06),
    }
}

/// How much of each of the twelve pitch classes the window contains.
fn chroma(spectra: &Spectra) -> [f32; 12] {
    let mut chroma = [0.0f32; 12];

    for frame in spectra.frames() {
        for (bin, magnitude) in frame.iter().enumerate() {
            let hz = spectra.frequency_of(bin);
            if !(LOWEST_HZ..=HIGHEST_HZ).contains(&hz) {
                continue;
            }
            chroma[pitch_class(hz)] += magnitude;
        }
    }

    chroma
}

/// Which of the twelve notes a frequency is nearest to.
fn pitch_class(hz: f32) -> usize {
    let semitones = 12.0 * (hz / A4_HZ).log2();
    let from_a = semitones.round() as i32;
    ((from_a + A_PITCH_CLASS as i32).rem_euclid(12)) as usize
}

/// Correlation between the chroma and a profile rotated to `tonic`.
///
/// Pearson's, so that a track being loud does not make it fit every key better.
fn fit(chroma: &[f32; 12], profile: &[f32; 12], tonic: usize) -> f32 {
    let chroma_mean = chroma.iter().sum::<f32>() / 12.0;
    let profile_mean = profile.iter().sum::<f32>() / 12.0;

    let mut covariance = 0.0;
    let mut chroma_spread = 0.0;
    let mut profile_spread = 0.0;

    for note in 0..12 {
        let heard = chroma[(note + tonic) % 12] - chroma_mean;
        let expected = profile[note] - profile_mean;
        covariance += heard * expected;
        chroma_spread += heard * heard;
        profile_spread += expected * expected;
    }

    let denominator = (chroma_spread * profile_spread).sqrt();
    if denominator <= f32::EPSILON {
        return 0.0;
    }
    covariance / denominator
}

#[cfg(test)]
mod tests {
    use super::{A4_HZ, KeyEstimate, measure, pitch_class};
    use crate::analysis::decode::Window;
    use crate::analysis::dsp::Spectra;
    use cadenza_core::domain::value_objects::Mode;

    /// A chord held for two seconds, given as semitone offsets from A3.
    fn chord(offsets: &[i32]) -> Spectra {
        let rate = 22_050u32;
        let samples = (0..rate * 2)
            .map(|index| {
                let time = index as f32 / rate as f32;
                offsets
                    .iter()
                    .map(|offset| {
                        let hz = (A4_HZ / 2.0) * 2.0f32.powf(*offset as f32 / 12.0);
                        (core::f32::consts::TAU * hz * time).sin() * 0.2
                    })
                    .sum()
            })
            .collect();
        Spectra::of(&Window { rate, samples })
    }

    #[test]
    fn a_note_is_named_by_where_it_sits_against_concert_pitch() {
        assert_eq!(pitch_class(A4_HZ), 9, "A is the tenth pitch class");
        assert_eq!(pitch_class(A4_HZ * 2.0), 9, "and so is the A above it");
        assert_eq!(pitch_class(261.63), 0, "middle C is the first");
        assert_eq!(pitch_class(392.0), 7, "G is the eighth");
    }

    #[test]
    fn a_major_triad_is_heard_as_a_major_key() {
        // A, C sharp, E: an A major chord.
        let estimate = measure(&chord(&[0, 4, 7]));
        let key = estimate.key.expect("a key");

        assert_eq!(key.mode(), Mode::Major, "a major triad is not minor");
        assert_eq!(key.pitch_class(), 9, "and it is A");
        assert!(estimate.strength > 0.0);
    }

    #[test]
    fn a_minor_triad_is_heard_as_a_minor_key() {
        // A, C, E.
        let key = measure(&chord(&[0, 3, 7])).key.expect("a key");
        assert_eq!(key.mode(), Mode::Minor);
        assert_eq!(key.pitch_class(), 9);
    }

    #[test]
    fn silence_is_in_no_key() {
        let quiet = Spectra::of(&Window {
            rate: 22_050,
            samples: vec![0.0; 22_050],
        });
        assert_eq!(measure(&quiet), KeyEstimate::NONE);
    }
}
