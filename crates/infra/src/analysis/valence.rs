//! How bright a track feels — and an admission about what that means.
//!
//! This is the one feature here that is not a measurement. Tempo is a period,
//! key is a correlation, loudness is an amplitude; musical positivity is a
//! judgement, and no amount of arithmetic over a spectrum turns into one.
//!
//! What this computes is the correlation a listener would recognise: major
//! keys, brisk tempos and open top ends feel happier than minor, slow and dark
//! ones. It is a *heuristic* and PROJECT_MASTER 10.4 uses it as one — a weight
//! among several in radio's ranking, never a label shown to anybody as fact.
//! Doing better would take a trained model, and 12.1 forbids one; doing worse
//! would be to leave the column empty and have M13 treat every track alike.

use cadenza_core::domain::value_objects::{Mode, MusicalKey};

use super::bpm::Tempo;
use super::spectral::Brightness;

/// The tempo either side of which a track starts to feel slow or hurried.
const NEUTRAL_BPM: f32 = 110.0;

/// How far from that the range extends.
const TEMPO_SPAN: f32 = 70.0;

/// How much of the answer each part is worth.
const MODE_WEIGHT: f32 = 0.4;
const TEMPO_WEIGHT: f32 = 0.35;
const BRIGHTNESS_WEIGHT: f32 = 0.25;

/// Estimates valence, `0.0..=1.0`.
pub fn measure(key: Option<MusicalKey>, tempo: &Tempo, brightness: &Brightness) -> f32 {
    // A key nobody could determine says nothing either way, so it contributes
    // the neutral half rather than pulling the answer down.
    let mode = match key.map(MusicalKey::mode) {
        Some(Mode::Major) => 1.0,
        Some(Mode::Minor) => 0.0,
        None => 0.5,
    };

    let pace = match tempo.bpm {
        Some(bpm) => (0.5 + (bpm - NEUTRAL_BPM) / (2.0 * TEMPO_SPAN)).clamp(0.0, 1.0),
        None => 0.5,
    };

    (mode * MODE_WEIGHT + pace * TEMPO_WEIGHT + brightness.centroid * BRIGHTNESS_WEIGHT)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::measure;
    use crate::analysis::bpm::Tempo;
    use crate::analysis::spectral::Brightness;
    use cadenza_core::domain::value_objects::{Mode, MusicalKey};

    fn tempo(bpm: f32) -> Tempo {
        Tempo {
            bpm: Some(bpm),
            confidence: 0.8,
            stability: 0.9,
            beat_strength: 0.7,
        }
    }

    fn brightness(centroid: f32) -> Brightness {
        Brightness {
            centroid,
            rolloff: centroid,
            drive: centroid,
        }
    }

    fn key(mode: Mode) -> Option<MusicalKey> {
        Some(MusicalKey::new(0, mode).expect("C"))
    }

    #[test]
    fn fast_bright_and_major_feels_happier_than_slow_dark_and_minor() {
        let bright = measure(key(Mode::Major), &tempo(140.0), &brightness(0.8));
        let bleak = measure(key(Mode::Minor), &tempo(70.0), &brightness(0.2));

        assert!(bright > 0.7, "{bright}");
        assert!(bleak < 0.3, "{bleak}");
    }

    #[test]
    fn each_ingredient_moves_the_answer_on_its_own() {
        let base = measure(key(Mode::Minor), &tempo(110.0), &brightness(0.5));

        assert!(measure(key(Mode::Major), &tempo(110.0), &brightness(0.5)) > base);
        assert!(measure(key(Mode::Minor), &tempo(160.0), &brightness(0.5)) > base);
        assert!(measure(key(Mode::Minor), &tempo(110.0), &brightness(0.9)) > base);
    }

    #[test]
    fn what_could_not_be_determined_is_neutral_rather_than_negative() {
        let unknown = measure(None, &Tempo::NONE, &brightness(0.5));
        assert!((unknown - 0.5).abs() < 0.01, "{unknown}");
    }
}
