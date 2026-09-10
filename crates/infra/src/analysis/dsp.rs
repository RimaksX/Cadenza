//! The transforms every feature is built on.
//!
//! One place for the short-time Fourier transform and the small statistics that
//! follow it, so that tempo, key, energy and brightness are all measured off the
//! same frames rather than each running its own slightly different analysis.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::decode::Window;

/// Samples per analysis frame.
///
/// At 22 050 Hz this is 93 ms and 10.8 Hz per bin: long enough to resolve the
/// pitches chroma is folded from, short enough that a drum hit stays an event
/// rather than becoming a smear.
pub const FRAME: usize = 2_048;

/// How far the window moves between frames.
///
/// A quarter of the frame, which is 23 ms — the resolution the tempo estimate
/// is limited by, and fine enough that a beat lands in a different frame from
/// the one before it at any tempo a listener would call music.
pub const HOP: usize = 512;

/// The magnitude spectra of a whole window, frame by frame.
pub struct Spectra {
    /// One row per frame, each `FRAME / 2` magnitudes.
    frames: Vec<Vec<f32>>,
    /// Samples per second of the audio these came from.
    rate: u32,
}

impl Spectra {
    /// Runs the transform over a window.
    ///
    /// Returns empty spectra for audio shorter than one frame rather than
    /// failing: a two-second file is a poor subject for analysis, not a
    /// malformed one, and every feature below has an answer for "nothing here".
    pub fn of(window: &Window) -> Self {
        let mut spectra = Self {
            frames: Vec::new(),
            rate: window.rate,
        };
        if window.samples.len() < FRAME {
            return spectra;
        }

        let mut planner = FftPlanner::new();
        let fft: Arc<dyn Fft<f32>> = planner.plan_fft_forward(FRAME);
        let taper = hann(FRAME);

        let mut buffer = vec![Complex32::new(0.0, 0.0); FRAME];
        let mut scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];

        for start in (0..=window.samples.len() - FRAME).step_by(HOP) {
            for (slot, (sample, weight)) in buffer
                .iter_mut()
                .zip(window.samples[start..start + FRAME].iter().zip(&taper))
            {
                *slot = Complex32::new(sample * weight, 0.0);
            }

            fft.process_with_scratch(&mut buffer, &mut scratch);

            // The upper half mirrors the lower for real input and carries no
            // information of its own.
            let magnitudes = buffer[..FRAME / 2]
                .iter()
                .map(|bin| bin.norm())
                .collect::<Vec<f32>>();
            spectra.frames.push(magnitudes);
        }

        spectra
    }

    /// How many frames there are.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when the audio was too short to transform.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The frames themselves.
    pub fn frames(&self) -> &[Vec<f32>] {
        &self.frames
    }

    /// The centre frequency of a bin, in hertz.
    pub fn frequency_of(&self, bin: usize) -> f32 {
        bin as f32 * self.rate as f32 / FRAME as f32
    }

    /// How much time one frame covers, in seconds.
    pub fn frame_seconds(&self) -> f32 {
        if self.rate == 0 {
            return 0.0;
        }
        HOP as f32 / self.rate as f32
    }
}

/// A raised cosine taper.
///
/// Without one, every frame ends in a discontinuity the transform reads as
/// broadband noise, and the brightness of a track would depend mostly on where
/// its frames happened to be cut.
pub fn hann(length: usize) -> Vec<f32> {
    if length <= 1 {
        return vec![1.0; length];
    }
    (0..length)
        .map(|index| {
            let phase = core::f32::consts::TAU * index as f32 / (length - 1) as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect()
}

/// Root mean square of a block of samples.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

/// The value below which `fraction` of a sorted-able set falls.
///
/// Used instead of a minimum and maximum, which in real recordings describe the
/// quietest click and the loudest transient rather than the quiet and loud
/// parts of the music.
pub fn percentile(values: &mut [f32], fraction: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);

    let position = (fraction.clamp(0.0, 1.0) * (values.len() - 1) as f32).round() as usize;
    values[position]
}

/// Squashes a value into `0.0..=1.0` around a midpoint.
///
/// Everything stored in `track_features` is normalised so that shuffle and
/// radio can weigh one number against another. A curve rather than a clamp, so
/// that the ends of the range stay distinguishable instead of piling up on 0
/// and 1.
pub fn squash(value: f32, midpoint: f32, width: f32) -> f32 {
    if !value.is_finite() || width <= 0.0 {
        return 0.0;
    }
    let scaled = (value - midpoint) / width;
    (0.5 + 0.5 * scaled.tanh()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{FRAME, HOP, Spectra, hann, percentile, rms, squash};
    use crate::analysis::decode::Window;

    /// A sine wave at `hz`, one second of it.
    fn tone(hz: f32, rate: u32) -> Window {
        let samples = (0..rate)
            .map(|index| (core::f32::consts::TAU * hz * index as f32 / rate as f32).sin() * 0.5)
            .collect();
        Window { rate, samples }
    }

    #[test]
    fn the_taper_starts_and_ends_at_nothing() {
        let taper = hann(8);
        assert!(taper[0].abs() < 1e-6);
        assert!(taper[7].abs() < 1e-6);
        assert!((taper[4] - 1.0).abs() < 0.2, "and peaks in the middle");
        assert_eq!(hann(1), vec![1.0], "a taper of one is no taper");
    }

    #[test]
    fn a_tone_lands_in_the_bin_it_belongs_to() {
        let window = tone(1_000.0, 22_050);
        let spectra = Spectra::of(&window);

        assert!(!spectra.is_empty());
        assert_eq!(spectra.len(), (22_050 - FRAME) / HOP + 1);

        let frame = &spectra.frames()[spectra.len() / 2];
        let loudest = frame
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .expect("a spectrum")
            .0;

        let found = spectra.frequency_of(loudest);
        assert!(
            (found - 1_000.0).abs() < 20.0,
            "a kilohertz tone was found at {found} Hz"
        );
    }

    #[test]
    fn audio_shorter_than_a_frame_transforms_to_nothing() {
        let window = Window {
            rate: 22_050,
            samples: vec![0.1; FRAME - 1],
        };
        let spectra = Spectra::of(&window);

        assert!(spectra.is_empty());
        assert_eq!(spectra.len(), 0);
    }

    #[test]
    fn loudness_and_percentiles_read_what_is_there() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);

        let mut values = [0.0, 1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&mut values, 0.0), 0.0);
        assert_eq!(percentile(&mut values, 1.0), 4.0);
        assert_eq!(percentile(&mut values, 0.5), 2.0);
        assert_eq!(percentile(&mut [], 0.5), 0.0);
    }

    #[test]
    fn squashing_keeps_the_ends_apart() {
        assert!((squash(5.0, 5.0, 2.0) - 0.5).abs() < 1e-6);
        assert!(squash(1.0, 5.0, 2.0) < 0.2);
        assert!(squash(9.0, 5.0, 2.0) > 0.8);
        assert!(squash(1_000.0, 5.0, 2.0) <= 1.0);
        assert_eq!(squash(f32::NAN, 5.0, 2.0), 0.0);
        assert_eq!(squash(1.0, 5.0, 0.0), 0.0);
    }
}
