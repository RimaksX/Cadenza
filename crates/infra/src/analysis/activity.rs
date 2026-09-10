//! How much is happening in a track, as opposed to how loud it is.
//!
//! Energy used to be two parts level and one part brightness, and on a real
//! library that is mostly a measurement of the mastering. Measured over the
//! owner's forty-one files: a brickwalled orchestral piece - bowed strings and
//! a small choir, as calm as the library gets - came out at 0.77 energy, above
//! three quarters of it, while a bass-led electronic remix with a clear beat
//! came out at 0.64, below a quarter of it. Radio put the first in Gaming and
//! the second in Sleep, which is exactly backwards.
//!
//! Spotify names five things behind its own `energy`: dynamic range, perceived
//! loudness, timbre, **onset rate** and general entropy. Two of those were
//! missing here, and they are the two that separate a sustained bowed note from
//! a kick drum. This module measures them.
//!
//! The detection function is the high-frequency-content flux Essentia
//! recommends for percussive events: the rise in each bin from one frame to the
//! next, weighted by the bin's index. Weighting by frequency is what keeps the
//! slow swell of a low sustained note from reading as an attack, and it is why
//! this is not the same envelope the tempo estimate autocorrelates.

use super::dsp::{Spectra, squash};

/// The share of frames that must be attacks for a track to count as busy.
///
/// Measured over the owner's library, where the share runs from 0.059 for the
/// orchestral piece to 0.165 for the busiest: the midpoint is the middle of
/// that, and the width spreads the two ends across the scale instead of
/// saturating them.
const TYPICAL_ATTACK_SHARE: f32 = 0.12;
/// How far either side of typical fills the range.
const ATTACK_SPAN: f32 = 0.04;

/// How much a frame must exceed the track's own average to be an attack.
const ATTACK_FACTOR: f32 = 2.0;

/// The spread of the flux that counts as ordinary, as a coefficient of
/// variation. Measured range over the same library: 0.62 to 2.35.
const TYPICAL_VARIATION: f32 = 1.3;
/// How far either side of that fills the range.
const VARIATION_SPAN: f32 = 0.6;

/// What a track's surface is doing, normalised to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Activity {
    /// How much of the track is attack rather than sustain.
    ///
    /// Spotify's "onset rate", measured as a share of frames rather than as
    /// events per second: a share needs no opinion about how long a track is,
    /// and the two orderings came out the same on the library they were
    /// measured against.
    pub onsets: f32,
    /// How much events stand out from the rest.
    ///
    /// Spotify's "general entropy", from the other end: music where everything
    /// happens at once has a flat flux, and music made of separate events has a
    /// spiky one.
    pub variation: f32,
}

impl Activity {
    /// What silence, or anything too short to measure, comes to.
    pub const STILL: Self = Self {
        onsets: 0.0,
        variation: 0.0,
    };
}

/// Measures how busy a window is.
pub fn measure(spectra: &Spectra) -> Activity {
    let flux = high_frequency_flux(spectra);
    if flux.len() < 2 {
        return Activity::STILL;
    }

    let mean = flux.iter().sum::<f32>() / flux.len() as f32;
    if mean <= 0.0 {
        return Activity::STILL;
    }

    let attacks = flux
        .iter()
        .filter(|value| **value > mean * ATTACK_FACTOR)
        .count() as f32
        / flux.len() as f32;

    let variance = flux.iter().map(|value| (value - mean).powi(2)).sum::<f32>() / flux.len() as f32;

    Activity {
        onsets: squash(attacks, TYPICAL_ATTACK_SHARE, ATTACK_SPAN),
        variation: squash(variance.sqrt() / mean, TYPICAL_VARIATION, VARIATION_SPAN),
    }
}

/// How much new sound arrived each frame, weighted towards the top.
///
/// Only rises count, for the same reason the tempo envelope only counts rises:
/// a note stopping is not an event. The bin index is the weight, which is the
/// high-frequency-content function - a cymbal or a stick moves the top of the
/// spectrum and a bowed note does not.
fn high_frequency_flux(spectra: &Spectra) -> Vec<f32> {
    spectra
        .frames()
        .windows(2)
        .map(|pair| {
            pair[1]
                .iter()
                .zip(&pair[0])
                .enumerate()
                .map(|(bin, (now, before))| bin as f32 * (now - before).max(0.0))
                .sum()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Activity, measure};
    use crate::analysis::decode::Window;
    use crate::analysis::dsp::Spectra;

    /// A held note: one frequency, never changing.
    fn sustained(rate: u32, seconds: u32) -> Window {
        let total = (rate * seconds) as usize;
        let samples = (0..total)
            .map(|index| (core::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin() * 0.5)
            .collect();
        Window { rate, samples }
    }

    /// The same note, chopped into hits.
    fn struck(rate: u32, seconds: u32, per_second: u32) -> Window {
        let total = (rate * seconds) as usize;
        let period = (rate / per_second) as usize;
        let samples = (0..total)
            .map(|index| {
                let into = index % period;
                // A hard attack and a quick decay, which is what a struck
                // instrument does and a bowed one does not.
                let envelope = (1.0 - into as f32 / (period as f32 * 0.3)).max(0.0);
                let mut state = (index as u32)
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                state ^= state >> 16;
                let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                noise * envelope * 0.5
            })
            .collect();
        Window { rate, samples }
    }

    #[test]
    fn a_struck_note_is_busier_than_a_held_one() {
        let rate = 22_050;
        let held = measure(&Spectra::of(&sustained(rate, 8)));
        let hit = measure(&Spectra::of(&struck(rate, 8, 4)));

        assert!(
            hit.onsets > held.onsets,
            "four hits a second against a held note: {} vs {}",
            hit.onsets,
            held.onsets
        );
        assert!(
            hit.variation > held.variation,
            "and the events stand out: {} vs {}",
            hit.variation,
            held.variation
        );
    }

    #[test]
    fn silence_measures_nothing_and_does_not_divide_by_it() {
        let silent = Window {
            rate: 22_050,
            samples: vec![0.0; 22_050 * 4],
        };
        assert_eq!(measure(&Spectra::of(&silent)), Activity::STILL);
        assert_eq!(
            measure(&Spectra::of(&Window {
                rate: 22_050,
                samples: Vec::new()
            })),
            Activity::STILL
        );
    }

    #[test]
    fn the_answers_always_fit_the_column() {
        let rate = 22_050;
        for window in [sustained(rate, 4), struck(rate, 4, 8)] {
            let activity = measure(&Spectra::of(&window));
            assert!((0.0..=1.0).contains(&activity.onsets));
            assert!((0.0..=1.0).contains(&activity.variation));
        }
    }
}
