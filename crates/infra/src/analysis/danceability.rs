//! How much a track invites movement.
//!
//! Three things a listener would agree on without being able to measure them: a
//! clear beat, a beat that stays where it is, and a tempo in the range bodies
//! move at. Nothing here claims to know whether the music is *good* to dance
//! to — only whether it is danceable at all.

use super::bpm::Tempo;

/// The tempo bodies move at most readily.
const IDEAL_BPM: f32 = 120.0;

/// How far from that a tempo can be before it stops helping.
const TEMPO_SPAN: f32 = 60.0;

/// Measures danceability from what the tempo estimate already found.
pub fn measure(tempo: &Tempo) -> f32 {
    let Some(bpm) = tempo.bpm else {
        // No pulse at all. Ambient music is not undanceable because it is
        // unpleasant; there is simply nothing to move to.
        return 0.0;
    };

    let in_range = (1.0 - (bpm - IDEAL_BPM).abs() / TEMPO_SPAN).clamp(0.0, 1.0);

    // Multiplied rather than averaged: a strong beat that wanders and a steady
    // pulse nobody can hear are both undanceable, and an average would call
    // them middling.
    (tempo.beat_strength * tempo.stability * in_range)
        .powf(1.0 / 3.0)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::measure;
    use crate::analysis::bpm::Tempo;

    fn tempo(bpm: f32, strength: f32, stability: f32) -> Tempo {
        Tempo {
            bpm: Some(bpm),
            confidence: 0.8,
            stability,
            beat_strength: strength,
        }
    }

    #[test]
    fn a_strong_steady_beat_at_a_walking_tempo_scores_highest() {
        let dancing = measure(&tempo(120.0, 0.9, 0.95));
        assert!(dancing > 0.8, "{dancing}");
    }

    #[test]
    fn every_missing_ingredient_costs() {
        let ideal = measure(&tempo(120.0, 0.9, 0.95));

        assert!(measure(&tempo(120.0, 0.1, 0.95)) < ideal, "a weak beat");
        assert!(measure(&tempo(120.0, 0.9, 0.2)) < ideal, "a wandering one");
        assert!(measure(&tempo(190.0, 0.9, 0.95)) < ideal, "too fast");
        assert!(measure(&tempo(62.0, 0.9, 0.95)) < ideal, "too slow");
    }

    #[test]
    fn music_with_no_pulse_is_not_danceable() {
        assert_eq!(measure(&Tempo::NONE), 0.0);
    }

    #[test]
    fn the_answer_always_fits_the_column() {
        for bpm in [40.0, 90.0, 120.0, 200.0] {
            for strength in [0.0, 0.5, 1.0] {
                let value = measure(&tempo(bpm, strength, 1.0));
                assert!((0.0..=1.0).contains(&value), "{value} for {bpm} BPM");
            }
        }
    }
}
