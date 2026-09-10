//! Content-based similarity for smart radio.
//!
//! The ranking. With history available:
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
//! The shape is fixed and the weights are not:
//!
//! ```text
//! score = w1*mood_match + w2*transition_score  + w3*user_preference
//!       + w4*freshness  + w5*diversity_penalty + w6*exploration_noise
//! ```
//!
//! The weights are settled in [`RankingWeights`]. Two things about the line as
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

/// What each term of the ranking is worth.
///
/// Calibration knobs, every one of them. The specification names the terms and
/// leaves the numbers to whoever has a library in front of them, so these are
/// ours and are meant to be argued with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingWeights {
    /// How much fitting the mood is worth. The largest, because the listener
    /// asked for a mood by name and everything else is a refinement of it.
    pub mood: f32,
    /// How much following the current track smoothly is worth.
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

/// Where the library's own values fall, so a band can mean a part of it.
///
/// The three `0.0..=1.0` features are the extractor's own invented scale, and
/// on a real library that scale collapses. Measured on the owner's 93 analysed
/// tracks: **danceability runs 0.87 / 0.95 / 0.98 across the quartiles and
/// energy 0.66 / 0.72 / 0.77.** Eighty per cent of the library sits inside a
/// tenth of the range. Read against absolute bands that is not a library with
/// quiet music in it — Focus (energy up to 0.45) and Sleep (up to 0.25) matched
/// nothing at all, while Driving (0.45 to 0.9) took eighty-three tracks of
/// ninety-three, and the moods stopped meaning different things.
///
/// So a value is scored by where it stands among the listener's own, not by the
/// number the extractor printed. Sleep becomes "as slow and as quiet as this
/// library has", which is what the mood's own comment always said it meant.
/// Two properties come free: a library where a feature is constant maps every
/// track to the middle rather than to an end, so a broken extractor makes a
/// term say nothing instead of saying something false; and a track cannot be in
/// the top tenth for energy and the bottom tenth at once, so moods that ask for
/// opposite things stop overlapping by construction.
///
/// **Tempo is left alone.** Beats per minute is a measurement in real units
/// that means the same thing in every library, and it measured well here — 65
/// to 159, with a median of 115. Ranking it would throw away the one feature
/// that did not need saving.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryScale {
    energy: Vec<f32>,
    valence: Vec<f32>,
    danceability: Vec<f32>,
}

impl LibraryScale {
    /// Reads the distribution of every analysed track.
    pub fn of<'a>(features: impl IntoIterator<Item = &'a TrackFeatures>) -> Self {
        let mut scale = Self::default();
        for row in features {
            scale.energy.push(row.energy);
            scale.valence.push(row.valence);
            scale.danceability.push(row.danceability);
        }
        for column in [
            &mut scale.energy,
            &mut scale.valence,
            &mut scale.danceability,
        ] {
            column.sort_by(f32::total_cmp);
        }
        scale
    }

    /// Where `value` stands among `sorted`, `0.0..=1.0`.
    ///
    /// The mid-rank: half the ties count as below and half as above, so a
    /// column with the same number in every row puts every track at 0.5 instead
    /// of all of them at one end.
    ///
    /// An empty column means there is no library to compare against — a unit
    /// test of a band, or a profile with nothing analysed — and then the value
    /// stands for itself, which is what it did before there was a scale.
    fn place(sorted: &[f32], value: f32) -> f32 {
        if sorted.is_empty() || !value.is_finite() {
            return value;
        }
        let below = sorted.partition_point(|other| *other < value);
        let up_to = sorted.partition_point(|other| *other <= value);
        (below + up_to) as f32 / (2.0 * sorted.len() as f32)
    }
}

/// How well a track fits a mood, `0.0..=1.0`.
///
/// The mean of the bands the mood actually states. A mood that states nothing
/// fits everything — it is a listener asking for flow rather than for a
/// character — and a track nobody has analysed fits neutrally rather than
/// badly, so that an unanalysed library still produces radio instead of
/// silence.
///
/// `scale` is the library the track is being judged against; see
/// [`LibraryScale`] for why the judging is relative.
pub fn mood_score(
    rules: &MoodRules,
    features: Option<&TrackFeatures>,
    scale: &LibraryScale,
) -> f32 {
    if rules.is_unconstrained() {
        return 1.0;
    }
    let Some(features) = features else {
        return NEUTRAL_SCORE;
    };

    let mut total = 0.0;
    let mut counted = 0.0f32;
    let mut met = 0.0f32;

    let mut term = |band: Option<FeatureBand>, value: Option<f32>| {
        if let Some(band) = band {
            // A stated band with nothing to measure against is the one case
            // that scores neutrally rather than zero: the track is not known to
            // be wrong, it is simply not known.
            let fit = value.map_or(NEUTRAL_SCORE, |value| band.fits(value));
            total += fit;
            counted += 1.0;
            if fit > 0.0 {
                met += 1.0;
            }
        }
    };

    term(rules.bpm, features.bpm.map(Bpm::as_f32));
    term(
        rules.energy,
        Some(LibraryScale::place(&scale.energy, features.energy)),
    );
    term(
        rules.valence,
        Some(LibraryScale::place(&scale.valence, features.valence)),
    );
    term(
        rules.danceability,
        Some(LibraryScale::place(
            &scale.danceability,
            features.danceability,
        )),
    );

    if counted == 0.0 {
        return 1.0;
    }

    // **Every stated band has to be met at all. One outright miss and the
    // track is not in this mood.**
    //
    // A band missed outright is not "a bit wrong" - it is a band with a soft
    // edge already built in, and the track fell past even that. Scoring it
    // proportionally was tried twice. First the plain mean, which put a 141 BPM
    // track fourth in Sleep; then the mean scaled by the
    // share of bands met, which is what shipped and which still let a 105 BPM
    // rap track score 0.33 in a mood whose tempo band ends at 80 - `t=0.00`
    // costing only a third.
    //
    // Measured across the owner's 41 tracks, the scaled mean left **41 of 41**
    // candidates alive in Driving, Focus and Morning: the station was the
    // library with the order shuffled. Requiring every band brings those to 33,
    // 18 and 28.
    //
    // 138 rejected zeroing because it "flattened every ranking underneath and
    // left a station with nothing to order its fallbacks by". That was true
    // while a zero-scoring track still competed on the other 60% of the rank.
    // It no longer does: `RadioService` drops zero-mood candidates outright, so
    // there is no ranking underneath left to flatten. The two changes only work
    // as a pair.
    if met < counted {
        return 0.0;
    }

    (total / counted).clamp(0.0, 1.0)
}

/// The final score a candidate is ranked by.
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

    use super::{LibraryScale, RankingWeights, mood_score, rank};
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

    /// No library to be ranked against, so a value stands for itself - which
    /// is what a test about a band is asking about.
    fn raw() -> LibraryScale {
        LibraryScale::default()
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
        let score = mood_score(&energetic(), Some(&features(Some(150.0), 0.8, 0.5)), &raw());
        assert!((score - 1.0).abs() < 1e-6, "got {score}");
    }

    #[test]
    fn a_track_the_mood_did_not_ask_for_scores_badly() {
        let lullaby = mood_score(&energetic(), Some(&features(Some(60.0), 0.1, 0.3)), &raw());
        assert!(lullaby < 0.1, "got {lullaby}");
    }

    #[test]
    fn one_band_missed_outright_is_the_whole_answer() {
        // The track the owner reported: 105 BPM against Sleep's 40-80, and
        // right about everything else. Under the shipped scheme its two good
        // bands carried it to 0.33 and into the station.
        let sleep = MoodRules {
            bpm: Some(FeatureBand::new(40.0, 80.0, 15.0)),
            energy: Some(FeatureBand::new(0.0, 0.25, 0.15)),
            danceability: Some(FeatureBand::new(0.0, 0.35, 0.2)),
            ..MoodRules::default()
        };
        let too_fast = features(Some(105.0), 0.1, 0.1);
        assert_eq!(mood_score(&sleep, Some(&too_fast), &raw()), 0.0);

        // And the gate is a gate, not a general souring: bend every band
        // without breaking one and the score is still the mean of the bends.
        // 78 BPM and 0.1 energy sit inside their bands; 0.4 danceability is
        // past the 0.35 top and inside the 0.2 of falloff after it.
        let slow = TrackFeatures {
            danceability: 0.4,
            ..features(Some(78.0), 0.1, 0.1)
        };
        let bent = mood_score(&sleep, Some(&slow), &raw());
        assert!((bent - (1.0 + 1.0 + 0.75) / 3.0).abs() < 1e-6, "got {bent}");
    }

    #[test]
    fn a_mood_that_asks_for_nothing_is_happy_with_anything() {
        let anything = MoodRules::default();
        assert_eq!(mood_score(&anything, None, &raw()), 1.0);
        assert_eq!(
            mood_score(&anything, Some(&features(Some(60.0), 0.1, 0.1)), &raw()),
            1.0
        );
    }

    #[test]
    fn a_track_nobody_has_analysed_is_neither_favoured_nor_blacklisted() {
        let unknown = mood_score(&energetic(), None, &raw());
        assert!(
            (unknown - 0.5).abs() < 1e-6,
            "an unanalysed library must still make radio, got {unknown}"
        );
    }

    #[test]
    fn a_missing_tempo_costs_only_the_term_it_belongs_to() {
        // Energy right, tempo unknown: half the terms are perfect and half are
        // neutral, so the answer sits between them rather than at either end.
        let score = mood_score(&energetic(), Some(&features(None, 0.8, 0.5)), &raw());
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

    #[test]
    fn a_value_is_placed_by_how_much_of_the_library_is_below_it() {
        let library: Vec<TrackFeatures> = [0.1, 0.2, 0.3, 0.4]
            .into_iter()
            .map(|energy| features(None, energy, 0.5))
            .collect();
        let scale = LibraryScale::of(&library);

        // The mid-rank: below the lowest of four is an eighth, above the
        // highest is seven eighths, and the two in between are evenly spread.
        let placed: Vec<f32> = library
            .iter()
            .map(|row| LibraryScale::place(&scale.energy, row.energy))
            .collect();
        assert_eq!(placed, vec![0.125, 0.375, 0.625, 0.875]);
    }

    #[test]
    fn a_feature_that_never_varies_says_nothing_instead_of_everything() {
        // What the owner's danceability column looks like: 0.87 / 0.95 / 0.98
        // across the quartiles, which read absolutely means every track is at
        // the top of the scale. Ranked, they are all in the middle of a
        // distribution that has no shape - which is the truth about it.
        let library: Vec<TrackFeatures> = std::iter::repeat_n(0.95, 5)
            .map(|dance| TrackFeatures {
                danceability: dance,
                ..features(None, 0.5, 0.5)
            })
            .collect();
        let scale = LibraryScale::of(&library);

        assert_eq!(LibraryScale::place(&scale.danceability, 0.95), 0.5);
    }

    #[test]
    fn opposite_moods_cannot_both_want_the_same_track() {
        // Read absolutely, this library is loud: every track sits inside
        // Driving's energy band and none inside Focus's, so one mood took
        // everything and the other took nothing. Ranked,
        // the quietest of them is the quiet one.
        let library: Vec<TrackFeatures> = [0.62, 0.66, 0.72, 0.77, 0.82]
            .into_iter()
            .map(|energy| features(Some(100.0), energy, 0.5))
            .collect();
        let scale = LibraryScale::of(&library);

        let quiet = MoodRules {
            energy: Some(FeatureBand::new(0.0, 0.45, 0.2)),
            ..MoodRules::default()
        };
        let loud = MoodRules {
            energy: Some(FeatureBand::new(0.65, 1.0, 0.25)),
            ..MoodRules::default()
        };

        let quietest = &library[0];
        let loudest = &library[4];

        assert!(
            mood_score(&quiet, Some(quietest), &scale) > mood_score(&quiet, Some(loudest), &scale),
            "the quiet mood wants the quietest of them"
        );
        assert!(
            mood_score(&loud, Some(loudest), &scale) > mood_score(&loud, Some(quietest), &scale),
            "and the loud mood wants the loudest"
        );
        assert!(
            mood_score(&quiet, Some(loudest), &scale) < 0.5,
            "which is what the two moods disagreeing looks like"
        );
    }
}
