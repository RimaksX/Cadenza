//! How well one track follows another.
//!
//! Implements the transition score of PROJECT_MASTER 9.3:
//!
//! ```text
//! transition_score = 0.35 * bpm_score
//!                  + 0.25 * key_score
//!                  + 0.25 * energy_score
//!                  + 0.15 * valence_score
//! ```

use super::NEUTRAL_SCORE;
use crate::domain::track::TrackFeatures;
use crate::domain::value_objects::Bpm;

/// Weight of the tempo term.
pub const BPM_WEIGHT: f32 = 0.35;
/// Weight of the key term.
pub const KEY_WEIGHT: f32 = 0.25;
/// Weight of the energy term.
pub const ENERGY_WEIGHT: f32 = 0.25;
/// Weight of the valence term.
pub const VALENCE_WEIGHT: f32 = 0.15;

/// Tempo difference, in BPM, at which the tempo term reaches zero.
///
/// A calibration knob, not a specified constant. 30 BPM is roughly the gap
/// between a slow house track and a fast one; beyond that the transition reads
/// as a deliberate gear change rather than a smooth continuation. Lower it for
/// stricter beat matching, raise it for more variety.
pub const MAX_BPM_DELTA: f32 = 30.0;

/// Tempo similarity, `0.0..=1.0`.
fn bpm_score(from: Option<Bpm>, to: Option<Bpm>) -> f32 {
    match (from, to) {
        (Some(from), Some(to)) => 1.0 - (from.delta(to) / MAX_BPM_DELTA).clamp(0.0, 1.0),
        _ => NEUTRAL_SCORE,
    }
}

/// Key compatibility, `0.0..=1.0`.
fn key_score(from: &TrackFeatures, to: &TrackFeatures) -> f32 {
    match (from.key, to.key) {
        (Some(from), Some(to)) => from.compatibility(to),
        _ => NEUTRAL_SCORE,
    }
}

/// Closeness of two normalised `0.0..=1.0` features.
fn closeness(from: f32, to: f32) -> f32 {
    if !from.is_finite() || !to.is_finite() {
        return NEUTRAL_SCORE;
    }
    1.0 - (from.clamp(0.0, 1.0) - to.clamp(0.0, 1.0)).abs()
}

/// How smoothly `to` follows `from`, `0.0..=1.0`.
///
/// Features that have not been analysed contribute [`NEUTRAL_SCORE`] rather than
/// zero, so an unanalysed library still produces usable orderings instead of
/// ranking everything as maximally jarring.
pub fn transition_score(from: &TrackFeatures, to: &TrackFeatures) -> f32 {
    let score = BPM_WEIGHT * bpm_score(from.bpm, to.bpm)
        + KEY_WEIGHT * key_score(from, to)
        + ENERGY_WEIGHT * closeness(from.energy, to.energy)
        + VALENCE_WEIGHT * closeness(from.valence, to.valence);
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        BPM_WEIGHT, ENERGY_WEIGHT, KEY_WEIGHT, MAX_BPM_DELTA, VALENCE_WEIGHT, transition_score,
    };
    use crate::domain::ids::MediaFileId;
    use crate::domain::track::TrackFeatures;
    use crate::domain::value_objects::{Bpm, Mode, MusicalKey, Timestamp};

    fn features(
        bpm: Option<f32>,
        key: Option<(u8, Mode)>,
        energy: f32,
        valence: f32,
    ) -> TrackFeatures {
        TrackFeatures {
            media_file_id: MediaFileId::new(),
            bpm: bpm.map(|value| Bpm::new(value).expect("in range")),
            bpm_confidence: 1.0,
            key: key.map(|(pitch, mode)| MusicalKey::new(pitch, mode).expect("in range")),
            energy,
            loudness: 0.5,
            spectral_centroid: 0.5,
            spectral_rolloff: 0.5,
            danceability: 0.5,
            valence,
            tempo_stability: 1.0,
            dynamic_range: 0.5,
            extractor_version: "test".to_owned(),
            analyzed_at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_weights_sum_to_one() {
        let total = BPM_WEIGHT + KEY_WEIGHT + ENERGY_WEIGHT + VALENCE_WEIGHT;
        assert!(
            (total - 1.0).abs() < 1e-6,
            "weights must sum to 1.0, got {total}"
        );
    }

    #[test]
    fn a_track_transitions_perfectly_into_its_own_twin() {
        let track = features(Some(120.0), Some((0, Mode::Major)), 0.7, 0.6);
        let twin = features(Some(120.0), Some((0, Mode::Major)), 0.7, 0.6);
        assert!(
            (transition_score(&track, &twin) - 1.0).abs() < 1e-6,
            "identical features must score 1.0"
        );
    }

    #[test]
    fn a_jarring_transition_scores_low() {
        let calm = features(Some(70.0), Some((0, Mode::Major)), 0.1, 0.2);
        let frantic = features(Some(175.0), Some((6, Mode::Minor)), 0.95, 0.9);

        let score = transition_score(&calm, &frantic);
        assert!(score < 0.15, "expected a poor score, got {score}");
    }

    #[test]
    fn a_smooth_transition_beats_a_jarring_one() {
        let anchor = features(Some(120.0), Some((0, Mode::Major)), 0.6, 0.5);
        let close = features(Some(124.0), Some((7, Mode::Major)), 0.65, 0.55);
        let distant = features(Some(200.0), Some((6, Mode::Minor)), 0.05, 0.95);

        assert!(transition_score(&anchor, &close) > transition_score(&anchor, &distant));
    }

    #[test]
    fn tempo_beyond_the_knob_contributes_nothing_but_does_not_go_negative() {
        let slow = features(Some(60.0), None, 0.5, 0.5);
        let fast = features(Some(60.0 + MAX_BPM_DELTA * 3.0), None, 0.5, 0.5);

        let score = transition_score(&slow, &fast);
        assert!(
            (0.0..=1.0).contains(&score),
            "score escaped its range: {score}"
        );
    }

    #[test]
    fn unanalysed_tracks_score_neutrally_rather_than_terribly() {
        let unknown = features(None, None, 0.5, 0.5);
        let other_unknown = features(None, None, 0.5, 0.5);

        let score = transition_score(&unknown, &other_unknown);
        assert!(
            score > 0.5,
            "missing tempo and key must not read as maximum incompatibility, got {score}"
        );
    }
}
