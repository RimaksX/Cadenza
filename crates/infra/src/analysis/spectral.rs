//! Where a track's energy sits in the spectrum.

use super::dsp::Spectra;

/// The lowest frequency the normalisation treats as "dark".
///
/// Below this is bass, and no music has its centre of mass there; mapping from
/// it rather than from zero is what stops every track scoring 0.05.
const FLOOR_HZ: f32 = 50.0;

/// Where the spectrum is split into "body" and "drive".
///
/// Roughly the top of the bass register: what is above it is what makes a mix
/// sound busy rather than merely loud.
const DRIVE_HZ: f32 = 500.0;

/// The share of energy below which rolloff is measured.
const ROLLOFF_SHARE: f32 = 0.85;

/// How a track sits in the spectrum, all normalised to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brightness {
    /// Centre of mass of the spectrum.
    pub centroid: f32,
    /// The frequency below which most of the energy lies.
    pub rolloff: f32,
    /// The share of energy above [`DRIVE_HZ`].
    pub drive: f32,
}

impl Brightness {
    /// What silence looks like: nothing anywhere, and no reason to prefer one
    /// end of the spectrum to the other.
    pub const SILENT: Self = Self {
        centroid: 0.0,
        rolloff: 0.0,
        drive: 0.0,
    };
}

/// Measures brightness over every frame of a window.
pub fn measure(spectra: &Spectra) -> Brightness {
    if spectra.is_empty() {
        return Brightness::SILENT;
    }

    let nyquist = spectra.frequency_of(spectra.frames()[0].len());
    let mut centroids = 0.0;
    let mut rolloffs = 0.0;
    let mut drives = 0.0;
    let mut counted = 0.0f32;

    for frame in spectra.frames() {
        let total: f32 = frame.iter().sum();
        // A silent frame has no centre of mass, and averaging in a zero would
        // drag a quiet track's brightness towards the bass.
        if total <= f32::EPSILON {
            continue;
        }

        let weighted: f32 = frame
            .iter()
            .enumerate()
            .map(|(bin, magnitude)| spectra.frequency_of(bin) * magnitude)
            .sum();

        let mut running = 0.0;
        let mut rolloff_bin = frame.len() - 1;
        for (bin, magnitude) in frame.iter().enumerate() {
            running += magnitude;
            if running >= total * ROLLOFF_SHARE {
                rolloff_bin = bin;
                break;
            }
        }

        let above: f32 = frame
            .iter()
            .enumerate()
            .filter(|(bin, _)| spectra.frequency_of(*bin) >= DRIVE_HZ)
            .map(|(_, magnitude)| magnitude)
            .sum();

        centroids += weighted / total;
        rolloffs += spectra.frequency_of(rolloff_bin);
        drives += above / total;
        counted += 1.0;
    }

    if counted == 0.0 {
        return Brightness::SILENT;
    }

    Brightness {
        centroid: place(centroids / counted, nyquist),
        rolloff: place(rolloffs / counted, nyquist),
        drive: (drives / counted).clamp(0.0, 1.0),
    }
}

/// Maps a frequency onto `0.0..=1.0` by octaves rather than by hertz.
///
/// Hearing is logarithmic and so is music: the distance from 200 Hz to 400 Hz
/// is the same musical distance as 2 kHz to 4 kHz, and a linear scale would put
/// almost every recording in the bottom fifth of the range.
fn place(hz: f32, nyquist: f32) -> f32 {
    if !hz.is_finite() || hz <= FLOOR_HZ || nyquist <= FLOOR_HZ {
        return 0.0;
    }
    let octaves = (hz / FLOOR_HZ).log2();
    let span = (nyquist / FLOOR_HZ).log2();
    (octaves / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{Brightness, measure, place};
    use crate::analysis::decode::Window;
    use crate::analysis::dsp::Spectra;

    fn tone(hz: f32) -> Spectra {
        let rate = 22_050;
        let samples = (0..rate * 2)
            .map(|index| (core::f32::consts::TAU * hz * index as f32 / rate as f32).sin() * 0.5)
            .collect();
        Spectra::of(&Window { rate, samples })
    }

    #[test]
    fn a_high_tone_is_brighter_than_a_low_one() {
        let low = measure(&tone(200.0));
        let high = measure(&tone(4_000.0));

        assert!(
            high.centroid > low.centroid,
            "{} should be brighter than {}",
            high.centroid,
            low.centroid
        );
        assert!(high.rolloff > low.rolloff);
        assert!(low.drive < 0.2, "a 200 Hz tone has no drive: {}", low.drive);
        assert!(high.drive > 0.8);
    }

    #[test]
    fn silence_has_no_brightness() {
        let quiet = Spectra::of(&Window {
            rate: 22_050,
            samples: vec![0.0; 22_050],
        });
        assert_eq!(measure(&quiet), Brightness::SILENT);
    }

    #[test]
    fn placing_is_by_octaves_and_stays_in_range() {
        let nyquist = 11_025.0;
        assert_eq!(place(10.0, nyquist), 0.0, "below the floor is nothing");
        assert_eq!(place(f32::NAN, nyquist), 0.0);
        assert!((place(nyquist, nyquist) - 1.0).abs() < 1e-6);

        // 800 Hz is four octaves above the floor of fifty, out of the eight
        // octaves the range spans: exactly half way up.
        assert!((place(800.0, nyquist) - 0.5).abs() < 0.02);
    }
}
