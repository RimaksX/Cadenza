//! Content-based similarity for smart radio.
//!
//! Implements PROJECT_MASTER 10.3. With history available:
//!
//! ```text
//! similarity = 0.60 * feature_cosine
//!            + 0.15 * genre_affinity
//!            + 0.15 * artist_affinity
//!            + 0.10 * user_preference
//! ```
//!
//! and without it, the user-preference term is redistributed:
//!
//! ```text
//! similarity = 0.75 * feature_cosine
//!            + 0.15 * genre_affinity
//!            + 0.10 * artist_affinity
//! ```
//!
//! # The ranking
//!
//! Section 10.4 gives the shape and leaves the weights open:
//!
//! ```text
//! score = w1*mood_match + w2*transition_score  + w3*user_preference
//!       + w4*freshness  + w5*diversity_penalty + w6*exploration_noise
//! ```
//!
//! M13 settles them, in [`RankingWeights`]. Two things about the line as
//! written had to be decided rather than copied. The diversity term
//! **subtracts** — as an addition it would reward a monotonous pick, which is
//! the opposite of what it is named for. And the noise is an *amplitude*
//! rather than a weight on a term: there is nothing to weigh, only a decision
//! about how far chance may move a candidate.

use super::NEUTRAL_SCORE;
use crate::domain::mood::{FeatureBand, MoodRules};
use crate::domain::track::TrackFeatures;
use crate::domain::value_objects::Bpm;

/// Weight of feature similarity when the listener has history.
pub const FEATURE_WEIGHT: f32 = 0.60;
/// Weight of genre affinity when the listener has history.
pub const GENRE_WEIGHT: f32 = 0.15;
/// Weight of artist affinity when the listener has history.
pub const ARTIST_WEIGHT: f32 = 0.15;
/// Weight of learned preference when the listener has history.
pub const PREFERENCE_WEIGHT: f32 = 0.10;

/// Weight of feature similarity when history is unavailable.
pub const FEATURE_WEIGHT_NO_HISTORY: f32 = 0.75;
/// Weight of genre affinity when history is unavailable.
pub const GENRE_WEIGHT_NO_HISTORY: f32 = 0.15;
/// Weight of artist affinity when history is unavailable.
pub const ARTIST_WEIGHT_NO_HISTORY: f32 = 0.10;

/// What each term of the ranking is worth (PROJECT_MASTER 10.4).
///
/// Calibration knobs, every one of them. The specification names the terms and
/// leaves the numbers to whoever has a library in front of them, so these are
/// ours and are meant to be argued with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingWeights {
    /// How much fitting the mood is worth. The largest, because the listener
    /// asked for a mood by name and everything else is a refinement of it.
    pub mood: f32,
    /// How much following the current track smoothly is worth (9.3).
    pub transition: f32,
    /// How much the listener's own verdicts are worth.
    pub preference: f32,
    /// How much not having heard it lately is worth.
    pub freshness: f32,
    /// How much a repeated artist is docked. Subtracted, not added.
    pub diversity: f32,
    /// How far chance may move a candidate either way.
    ///
    /// Without it, radio started twice from the same track in the same mood
    /// plays the same tracks in the same order, which is a playlist wearing
    /// radio's clothes.
    pub exploration: f32,
}

impl RankingWeights {
    /// What every mood ranks with until one asks for its own.
    ///
    /// The four rewarded terms sum to one, so a perfect candidate with no
    /// penalty scores one before noise. The penalty is a quarter, which is
    /// enough to push a same-artist track below a merely good one without
    /// banning it — that is the cooldown's job, and a weight that could ban
    /// would be a rule pretending to be a preference.
    pub const DEFAULT: Self = Self {
        mood: 0.40,
        transition: 0.25,
        preference: 0.15,
        freshness: 0.20,
        diversity: 0.25,
        exploration: 0.10,
    };
}

/// How well a track fits a mood, `0.0..=1.0`.
///
/// The mean of the bands the mood actually states. A mood that states nothing
/// fits everything — it is a listener asking for flow rather than for a
/// character — and a track nobody has analysed fits neutrally rather than
/// badly, so that an unanalysed library still produces radio instead of
/// silence.
pub fn mood_score(rules: &MoodRules, features: Option<&TrackFeatures>) -> f32 {
    if rules.is_unconstrained() {
        return 1.0;
    }
    let Some(features) = features else {
        return NEUTRAL_SCORE;
    };

    let mut total = 0.0;
    let mut counted = 0.0f32;

    let mut term = |band: Option<FeatureBand>, value: Option<f32>| {
        if let Some(band) = band {
            // A stated band with nothing to measure against is the one case
            // that scores neutrally rather than zero: the track is not known to
            // be wrong, it is simply not known.
            total += value.map_or(NEUTRAL_SCORE, |value| band.fits(value));
            counted += 1.0;
        }
    };

    term(rules.bpm, features.bpm.map(Bpm::as_f32));
    term(rules.energy, Some(features.energy));
    term(rules.valence, Some(features.valence));
    term(rules.danceability, Some(features.danceability));

    if counted == 0.0 {
        return 1.0;
    }
    (total / counted).clamp(0.0, 1.0)
}

/// The final score a candidate is ranked by (PROJECT_MASTER 10.4).
///
/// `noise` is `0.0..=1.0` from the caller's own source of chance, which keeps
/// this function pure and its ordering reproducible in a test.
///
/// The result is deliberately *not* clamped to `0.0..=1.0`: it exists to be
/// compared with other candidates' scores, and squashing the ends would make
/// two differently-bad candidates look identical.
pub fn rank(
    weights: &RankingWeights,
    mood: f32,
    transition: f32,
    preference: f32,
    freshness: f32,
    repeats_artist: bool,
    noise: f32,
) -> f32 {
    let noise = if noise.is_finite() {
        (noise.clamp(0.0, 1.0) - 0.5) * 2.0 * weights.exploration
    } else {
        0.0
    };

    weights.mood * clamp01(mood)
        + weights.transition * clamp01(transition)
        + weights.preference * clamp01(preference)
        + weights.freshness * clamp01(freshness)
        - if repeats_artist {
            weights.diversity
        } else {
            0.0
        }
        + noise
}

/// How similar a candidate is to what is playing, `0.0..=1.0`.
///
/// `user_preference` is `None` when the profile has history disabled. That is
/// the common case for a privacy-minded listener, not an error path: radio must
/// work fully without it, which is why the weights redistribute rather than the
/// term defaulting to zero. Defaulting to zero would drag every candidate's
/// score down by a flat 0.10 and make the whole scale meaningless.
///
/// All inputs are clamped, so an out-of-range affinity from a future scorer
/// cannot push the result outside `0.0..=1.0`.
pub fn similarity(
    feature_cosine: f32,
    genre_affinity: f32,
    artist_affinity: f32,
    user_preference: Option<f32>,
) -> f32 {
    let feature = clamp01(feature_cosine);
    let genre = clamp01(genre_affinity);
    let artist = clamp01(artist_affinity);

    let score = match user_preference {
        Some(preference) => {
            FEATURE_WEIGHT * feature
                + GENRE_WEIGHT * genre
                + ARTIST_WEIGHT * artist
                + PREFERENCE_WEIGHT * clamp01(preference)
        }
        None => {
            FEATURE_WEIGHT_NO_HISTORY * feature
                + GENRE_WEIGHT_NO_HISTORY * genre
                + ARTIST_WEIGHT_NO_HISTORY * artist
        }
    };
    score.clamp(0.0, 1.0)
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ARTIST_WEIGHT, ARTIST_WEIGHT_NO_HISTORY, FEATURE_WEIGHT, FEATURE_WEIGHT_NO_HISTORY,
        GENRE_WEIGHT, GENRE_WEIGHT_NO_HISTORY, PREFERENCE_WEIGHT, similarity,
    };

    #[test]
    fn both_weight_sets_sum_to_one() {
        let with_history = FEATURE_WEIGHT + GENRE_WEIGHT + ARTIST_WEIGHT + PREFERENCE_WEIGHT;
        let without =
            FEATURE_WEIGHT_NO_HISTORY + GENRE_WEIGHT_NO_HISTORY + ARTIST_WEIGHT_NO_HISTORY;

        assert!((with_history - 1.0).abs() < 1e-6, "got {with_history}");
        assert!((without - 1.0).abs() < 1e-6, "got {without}");
    }

    #[test]
    fn a_perfect_match_scores_one_either_way() {
        assert!((similarity(1.0, 1.0, 1.0, Some(1.0)) - 1.0).abs() < 1e-6);
        assert!((similarity(1.0, 1.0, 1.0, None) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn no_match_scores_zero_either_way() {
        assert!(similarity(0.0, 0.0, 0.0, Some(0.0)).abs() < 1e-6);
        assert!(similarity(0.0, 0.0, 0.0, None).abs() < 1e-6);
    }

    #[test]
    fn disabling_history_does_not_penalise_the_candidate() {
        let with_history = similarity(0.8, 0.5, 0.5, Some(0.8));
        let without = similarity(0.8, 0.5, 0.5, None);

        assert!(
            (with_history - without).abs() < 0.05,
            "a privacy-minded listener must not get systematically lower scores: \
             {with_history} vs {without}"
        );
    }

    #[test]
    fn features_dominate_the_score() {
        let strong_features = similarity(1.0, 0.0, 0.0, None);
        let strong_metadata = similarity(0.0, 1.0, 1.0, None);
        assert!(strong_features > strong_metadata);
    }

    #[test]
    fn out_of_range_and_non_finite_inputs_cannot_escape_the_scale() {
        for score in [
            similarity(5.0, 5.0, 5.0, Some(5.0)),
            similarity(-1.0, -1.0, -1.0, Some(-1.0)),
            similarity(f32::NAN, f32::INFINITY, 0.5, Some(f32::NAN)),
        ] {
            assert!((0.0..=1.0).contains(&score), "score escaped: {score}");
        }
    }

    use super::{RankingWeights, mood_score, rank};
    use crate::domain::ids::MediaFileId;
    use crate::domain::mood::{FeatureBand, MoodRules};
    use crate::domain::track::TrackFeatures;
    use crate::domain::value_objects::{Bpm, Timestamp};

    fn features(bpm: Option<f32>, energy: f32, valence: f32) -> TrackFeatures {
        TrackFeatures {
            media_file_id: MediaFileId::new(),
            bpm: bpm.map(|value| Bpm::new(value).expect("in range")),
            bpm_confidence: 1.0,
            key: None,
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

    /// Something like Workout: fast and loud.
    fn energetic() -> MoodRules {
        MoodRules {
            bpm: Some(FeatureBand::new(125.0, 175.0, 25.0)),
            energy: Some(FeatureBand::new(0.65, 1.0, 0.25)),
            ..MoodRules::default()
        }
    }

    #[test]
    fn a_track_the_mood_asked_for_scores_top_marks() {
        let score = mood_score(&energetic(), Some(&features(Some(150.0), 0.8, 0.5)));
        assert!((score - 1.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn a_track_the_mood_did_not_ask_for_scores_badly() {
        let lullaby = mood_score(&energetic(), Some(&features(Some(60.0), 0.1, 0.3)));
        assert!(lullaby < 0.1, "got {lullaby}");
    }

    #[test]
    fn a_mood_that_asks_for_nothing_is_happy_with_anything() {
        let anything = MoodRules::default();
        assert_eq!(mood_score(&anything, None), 1.0);
        assert_eq!(
            mood_score(&anything, Some(&features(Some(60.0), 0.1, 0.1))),
            1.0
        );
    }

    #[test]
    fn a_track_nobody_has_analysed_is_neither_favoured_nor_blacklisted() {
        let unknown = mood_score(&energetic(), None);
        assert!(
            (unknown - 0.5).abs() < 1e-6,
            "an unanalysed library must still make radio, got {unknown}"
        );
    }

    #[test]
    fn a_missing_tempo_costs_only_the_term_it_belongs_to() {
        // Energy right, tempo unknown: half the terms are perfect and half are
        // neutral, so the answer sits between them rather than at either end.
        let score = mood_score(&energetic(), Some(&features(None, 0.8, 0.5)));
        assert!((score - 0.75).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn the_rewarded_weights_sum_to_one() {
        let weights = RankingWeights::DEFAULT;
        let total = weights.mood + weights.transition + weights.preference + weights.freshness;
        assert!((total - 1.0).abs() < 1e-6, "got {total}");
    }

    #[test]
    fn a_perfect_candidate_scores_one_before_chance_moves_it() {
        let weights = RankingWeights::DEFAULT;
        let score = rank(&weights, 1.0, 1.0, 1.0, 1.0, false, 0.5);
        assert!((score - 1.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn repeating_an_artist_costs_the_diversity_weight() {
        let weights = RankingWeights::DEFAULT;
        let fresh = rank(&weights, 0.9, 0.9, 0.5, 1.0, false, 0.5);
        let repeat = rank(&weights, 0.9, 0.9, 0.5, 1.0, true, 0.5);

        assert!(
            (fresh - repeat - weights.diversity).abs() < 1e-6,
            "the penalty must subtract: {fresh} vs {repeat}"
        );
    }

    #[test]
    fn chance_moves_a_candidate_either_way_and_only_so_far() {
        let weights = RankingWeights::DEFAULT;
        let middle = rank(&weights, 0.5, 0.5, 0.5, 0.5, false, 0.5);
        let lucky = rank(&weights, 0.5, 0.5, 0.5, 0.5, false, 1.0);
        let unlucky = rank(&weights, 0.5, 0.5, 0.5, 0.5, false, 0.0);

        assert!(lucky > middle && middle > unlucky);
        assert!((lucky - middle - weights.exploration).abs() < 1e-6);
        assert!((middle - unlucky - weights.exploration).abs() < 1e-6);
    }

    #[test]
    fn a_mood_match_outweighs_a_smooth_transition() {
        let weights = RankingWeights::DEFAULT;
        let right_mood = rank(&weights, 1.0, 0.0, 0.5, 0.5, false, 0.5);
        let smooth_only = rank(&weights, 0.0, 1.0, 0.5, 0.5, false, 0.5);

        assert!(
            right_mood > smooth_only,
            "the listener asked for a mood by name: {right_mood} vs {smooth_only}"
        );
    }

    #[test]
    fn nonsense_terms_cannot_run_away_with_the_score() {
        let weights = RankingWeights::DEFAULT;
        let score = rank(&weights, 50.0, -20.0, f32::NAN, 3.0, false, f32::NAN);
        assert!(score.is_finite() && score <= 1.0, "got {score}");
    }
}
