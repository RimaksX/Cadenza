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
//! # What is deliberately not here
//!
//! The final ranking of section 10.4:
//!
//! ```text
//! score = w1*mood_match + w2*transition_score  + w3*user_preference
//!       + w4*freshness  + w5*diversity_penalty + w6*exploration_noise
//! ```
//!
//! It leaves `w1..w6` unspecified, and writes the diversity term as an *added*
//! penalty, which would raise the score of a monotonous pick rather than lower
//! it. Both need settling with a real library in front of us, so ranking lands
//! in M13. When it does, the diversity term must subtract.

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
}
