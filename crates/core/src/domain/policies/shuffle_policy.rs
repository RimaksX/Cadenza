//! What smart shuffle is allowed to play next, and which of those it picks.
//!
//! The hard rules came first; the choice itself came later. Score every
//! candidate against what is playing, keep the best handful, and pick from those
//! at random. Three sentences, and a great deal rides on each of them —
//!
//! - **score**, so that one track follows another musically rather than by
//!   accident. The formula lives in
//!   [`super::transition_policy::transition_score`].
//! - **keep the best handful** rather than the single best, because always
//!   playing the closest match makes a library of five thousand tracks sound
//!   like a library of forty.
//! - **at random**, weighted, because shuffle has to keep feeling like
//!   shuffle. A deterministic "best next track" is a playlist somebody else
//!   wrote.
//!
//! Everything here is pure. The seed and the features arrive as arguments, which
//! is what lets an ordering nobody can hear be tested by somebody who cannot
//! hear it either.

use crate::domain::ids::MediaFileId;
use crate::domain::media_file::FileState;
use crate::domain::track::TrackFeatures;

use super::transition_policy::transition_score;

/// How many recently played tracks an artist is barred from reappearing within.
///
/// A calibration knob. Too small and a three-track artist dominates a small
/// library; too large and shuffling a library of one artist stalls, which is why
/// the caller must be prepared to relax the constraint when no candidate passes.
pub const ARTIST_COOLDOWN: usize = 3;

/// How many top-scoring candidates the weighted random pick chooses among.
///
/// The top ten to twenty; this is where in that range it sits.
pub const CANDIDATE_POOL_SIZE: usize = 15;

/// Reorders a pool into a random permutation.
///
/// This is the whole of the basic shuffle. A permutation satisfies the first
/// hard rule by construction — every track plays once before any plays twice —
/// and says nothing about which order is *good*, which is what the preference
/// scoring adds.
///
/// The seed is a parameter because the domain has no entropy of its own, and
/// because a shuffle that cannot be reproduced cannot be tested.
pub fn shuffle<T>(pool: &mut [T], seed: u64) {
    // xorshift64: a few instructions, no dependency, and far better than a
    // library-order "shuffle" that only ever rotates. The modulo below is
    // biased by about one part in 2^58 for a pool of any size a listener will
    // ever have, which is not a musical problem.
    let mut state = seed | 1; // zero is xorshift's fixed point.

    for index in (1..pool.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let pick = (state % (index as u64 + 1)) as usize;
        pool.swap(index, pick);
    }
}

/// True when the artist appeared too recently to play again.
///
/// Tracks with no known artist never block each other: an unanalysed library
/// would otherwise be one giant cooldown group and shuffle would refuse to move.
/// By name rather than by identifier: what a listener notices is the same name
/// three times running, and a name is what every listing already carries.
pub fn artist_on_cooldown(candidate: Option<&str>, recently_played: &[Option<&str>]) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    recently_played
        .iter()
        .rev()
        .take(ARTIST_COOLDOWN)
        .any(|recent| *recent == Some(candidate))
}

/// True when a track may be chosen as the next one.
///
/// Enforces all three hard rules at once: no repeats until the pool is
/// exhausted, no artist within the cooldown, and no missing or unreadable files.
pub fn is_eligible(
    file_state: FileState,
    already_played: bool,
    candidate_artist: Option<&str>,
    recently_played_artists: &[Option<&str>],
) -> bool {
    file_state.is_playable()
        && !already_played
        && !artist_on_cooldown(candidate_artist, recently_played_artists)
}

/// One track shuffle may choose, and everything the choice needs to know.
///
/// Borrowed rather than owned: the caller has just read a listing and a table of
/// features, and copying five thousand of each to ask one question would be
/// work nobody hears.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The file that would play.
    pub media_file_id: MediaFileId,
    /// Whether the file is still there and readable.
    pub file_state: FileState,
    /// Who made it, as the listing shows it.
    pub artist: Option<&'a str>,
    /// What it sounds like, when it has been analysed.
    pub features: Option<&'a TrackFeatures>,
}

/// Chooses what plays after `current` from `candidates`.
///
/// The whole of the preference rule. `recently_played_artists` is the tail of
/// what has played, most recent last; `candidates` must already exclude what
/// has been heard this round, which is the first hard rule and the caller's to
/// enforce because only the caller knows what the round is.
///
/// Returns `None` only when there is nothing playable at all. A cooldown that
/// nobody can satisfy is relaxed rather than obeyed: a library of one artist
/// must still shuffle, and refusing to move is a worse answer than playing them
/// twice.
pub fn choose_next(
    current: Option<&TrackFeatures>,
    candidates: &[Candidate<'_>],
    recently_played_artists: &[Option<&str>],
    seed: u64,
) -> Option<MediaFileId> {
    let playable = |candidate: &&Candidate<'_>| candidate.file_state.is_playable();

    let mut eligible: Vec<&Candidate<'_>> = candidates
        .iter()
        .filter(playable)
        .filter(|candidate| !artist_on_cooldown(candidate.artist, recently_played_artists))
        .collect();

    if eligible.is_empty() {
        eligible = candidates.iter().filter(playable).collect();
    }
    if eligible.is_empty() {
        return None;
    }

    // Nothing to score against — the first track of a session, or a current
    // track nobody has analysed yet. Chance alone is the honest answer, and it
    // is also what the basic shuffle did.
    let Some(current) = current else {
        return pick(&eligible, &vec![1.0; eligible.len()], seed);
    };

    let mut scored: Vec<(f32, &Candidate<'_>)> = eligible
        .into_iter()
        .map(|candidate| {
            let score = match candidate.features {
                Some(features) => transition_score(current, features),
                // An unanalysed track is neither favoured nor blacklisted: it
                // sits mid-table and gets its turn, which is what keeps a
                // half-analysed library from playing only its analysed half.
                None => super::NEUTRAL_SCORE,
            };
            (score, candidate)
        })
        .collect();

    // Descending, then the best handful. `total_cmp` rather than `partial_cmp`
    // because a NaN in a sort comparator is a panic waiting for the one library
    // that has one.
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.truncate(CANDIDATE_POOL_SIZE);

    let pool: Vec<&Candidate<'_>> = scored.iter().map(|(_, candidate)| *candidate).collect();
    let weights: Vec<f32> = scored.iter().map(|(score, _)| *score).collect();
    pick(&pool, &weights, seed)
}

/// A weighted draw from `pool`.
///
/// Weighted rather than uniform so that a better transition is likelier without
/// being certain. Falls back to a uniform draw when every weight is zero, which
/// happens when nothing scores at all — an unanalysed library, mostly.
fn pick(pool: &[&Candidate<'_>], weights: &[f32], seed: u64) -> Option<MediaFileId> {
    if pool.is_empty() {
        return None;
    }

    let total: f32 = weights.iter().filter(|weight| weight.is_finite()).sum();
    if total <= 0.0 {
        let index = (seed % pool.len() as u64) as usize;
        return pool.get(index).map(|candidate| candidate.media_file_id);
    }

    // A point along the line made of the weights laid end to end.
    let target = (seed % 1_000_000) as f32 / 1_000_000.0 * total;
    let mut running = 0.0;
    for (candidate, weight) in pool.iter().zip(weights) {
        running += weight.max(0.0);
        if running >= target {
            return Some(candidate.media_file_id);
        }
    }

    // Only reachable through floating-point drift at the very end of the line.
    pool.last().map(|candidate| candidate.media_file_id)
}

#[cfg(test)]
mod tests {
    use super::{
        ARTIST_COOLDOWN, CANDIDATE_POOL_SIZE, Candidate, artist_on_cooldown, choose_next,
        is_eligible, shuffle,
    };
    use crate::domain::ids::MediaFileId;
    use crate::domain::media_file::FileState;
    use crate::domain::track::TrackFeatures;
    use crate::domain::value_objects::{Bpm, Mode, MusicalKey, Timestamp};

    #[test]
    fn shuffling_keeps_every_track_exactly_once() {
        let mut pool: Vec<u32> = (0..64).collect();
        shuffle(&mut pool, 0x5eed_1234_9abc_def0);

        let mut sorted = pool.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (0..64).collect::<Vec<_>>(),
            "nothing lost or doubled"
        );
        assert_ne!(
            pool,
            (0..64).collect::<Vec<_>>(),
            "and it did not stay in order"
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_order() {
        let mut once: Vec<u32> = (0..32).collect();
        let mut again = once.clone();
        shuffle(&mut once, 7);
        shuffle(&mut again, 7);
        assert_eq!(once, again);
    }

    #[test]
    fn a_zero_seed_still_shuffles() {
        // xorshift never leaves zero, so a seed of zero would return the pool
        // untouched — and a listener whose first shuffle does nothing has been
        // told the button is broken.
        let mut pool: Vec<u32> = (0..32).collect();
        shuffle(&mut pool, 0);
        assert_ne!(pool, (0..32).collect::<Vec<_>>());
    }

    #[test]
    fn short_pools_are_left_alone_rather_than_panicking() {
        let mut empty: [u32; 0] = [];
        shuffle(&mut empty, 1);

        let mut single = [42];
        shuffle(&mut single, 1);
        assert_eq!(single, [42]);
    }

    #[test]
    fn an_artist_just_played_is_on_cooldown() {
        let history = vec![Some("Someone Else"), Some("Portishead")];
        assert!(artist_on_cooldown(Some("Portishead"), &history));
    }

    #[test]
    fn the_cooldown_expires_after_enough_other_tracks() {
        let mut history = vec![Some("Portishead")];
        for name in ["Massive Attack", "Tricky", "Morcheeba", "Lamb"]
            .into_iter()
            .take(ARTIST_COOLDOWN)
        {
            history.push(Some(name));
        }
        assert!(!artist_on_cooldown(Some("Portishead"), &history));
    }

    #[test]
    fn unknown_artists_never_block_each_other() {
        let history = vec![None, None, None];
        assert!(
            !artist_on_cooldown(None, &history),
            "an unanalysed library must still shuffle"
        );
    }

    #[test]
    fn an_empty_history_blocks_nothing() {
        assert!(!artist_on_cooldown(Some("Portishead"), &[]));
    }

    #[test]
    fn every_hard_rule_can_veto_a_candidate() {
        let artist = "Portishead";
        let clean_history = vec![Some("Massive Attack")];

        assert!(is_eligible(
            FileState::Available,
            false,
            Some(artist),
            &clean_history
        ));

        assert!(
            !is_eligible(FileState::Missing, false, Some(artist), &clean_history),
            "missing files are excluded"
        );
        assert!(
            !is_eligible(FileState::Errored, false, Some(artist), &clean_history),
            "unreadable files are excluded"
        );
        assert!(
            !is_eligible(FileState::Available, true, Some(artist), &clean_history),
            "no repeats until the pool is exhausted"
        );
        assert!(
            !is_eligible(FileState::Available, false, Some(artist), &[Some(artist)]),
            "the artist cooldown applies"
        );
    }

    /// Features that differ only in the ways the score looks at.
    fn features(bpm: f32, energy: f32) -> TrackFeatures {
        TrackFeatures {
            media_file_id: MediaFileId::new(),
            bpm: Some(Bpm::new(bpm).expect("in range")),
            bpm_confidence: 1.0,
            key: Some(MusicalKey::new(0, Mode::Major).expect("C major")),
            energy,
            loudness: 0.5,
            spectral_centroid: 0.5,
            spectral_rolloff: 0.5,
            danceability: 0.5,
            valence: 0.5,
            tempo_stability: 1.0,
            dynamic_range: 0.5,
            extractor_version: "test".to_owned(),
            analyzed_at: Timestamp::UNIX_EPOCH,
        }
    }

    fn candidate<'a>(artist: Option<&'a str>, features: &'a TrackFeatures) -> Candidate<'a> {
        Candidate {
            media_file_id: features.media_file_id,
            file_state: FileState::Available,
            artist,
            features: Some(features),
        }
    }

    #[test]
    fn the_track_that_follows_best_is_the_one_most_often_chosen() {
        let current = features(120.0, 0.6);
        let close = features(122.0, 0.62);
        let distant = features(190.0, 0.05);
        let pool = [candidate(None, &close), candidate(None, &distant)];

        // Over many draws rather than one: the pick is weighted, not decided,
        // and a test that demanded the best every time would be testing for a
        // behaviour shuffle explicitly rules out.
        let mut chose_close = 0;
        for seed in 0..200u64 {
            if choose_next(Some(&current), &pool, &[], seed * 7_919) == Some(close.media_file_id) {
                chose_close += 1;
            }
        }

        assert!(
            chose_close > 120,
            "the smooth transition should win most of the time, won {chose_close} of 200"
        );
        assert!(
            chose_close < 200,
            "but not every time — shuffle has to keep feeling like shuffle"
        );
    }

    #[test]
    fn an_artist_inside_the_cooldown_is_passed_over() {
        let current = features(120.0, 0.6);
        let theirs = features(121.0, 0.6);
        let someone_else = features(180.0, 0.1);
        let pool = [
            candidate(Some("Portishead"), &theirs),
            candidate(Some("Autechre"), &someone_else),
        ];

        // The better transition is the one on cooldown, so this is the rule
        // overruling the score rather than agreeing with it.
        for seed in 0..20u64 {
            assert_eq!(
                choose_next(Some(&current), &pool, &[Some("Portishead")], seed * 104_729),
                Some(someone_else.media_file_id)
            );
        }
    }

    #[test]
    fn a_cooldown_nobody_can_satisfy_is_relaxed_rather_than_obeyed() {
        let current = features(120.0, 0.6);
        let only = features(121.0, 0.6);
        let pool = [candidate(Some("Portishead"), &only)];

        assert_eq!(
            choose_next(Some(&current), &pool, &[Some("Portishead")], 1),
            Some(only.media_file_id),
            "a library of one artist must still shuffle"
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_never_chosen() {
        let current = features(120.0, 0.6);
        let missing = features(120.0, 0.6);
        let present = features(190.0, 0.05);

        let pool = [
            Candidate {
                media_file_id: missing.media_file_id,
                file_state: FileState::Missing,
                artist: None,
                features: Some(&missing),
            },
            candidate(None, &present),
        ];

        for seed in 0..20u64 {
            assert_eq!(
                choose_next(Some(&current), &pool, &[], seed * 65_537),
                Some(present.media_file_id)
            );
        }
    }

    #[test]
    fn an_unanalysed_library_still_shuffles() {
        let tracks: Vec<TrackFeatures> = (0..5).map(|_| features(120.0, 0.5)).collect();
        let pool: Vec<Candidate<'_>> = tracks
            .iter()
            .map(|track| Candidate {
                media_file_id: track.media_file_id,
                file_state: FileState::Available,
                artist: None,
                features: None,
            })
            .collect();

        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..200u64 {
            let chosen = choose_next(None, &pool, &[], seed * 7_919).expect("something to play");
            seen.insert(chosen);
        }

        assert_eq!(seen.len(), 5, "every track was reachable");
    }

    #[test]
    fn nothing_playable_means_nothing_to_play() {
        assert_eq!(choose_next(None, &[], &[], 1), None);

        let gone = features(120.0, 0.5);
        let pool = [Candidate {
            media_file_id: gone.media_file_id,
            file_state: FileState::Missing,
            artist: None,
            features: Some(&gone),
        }];
        assert_eq!(choose_next(None, &pool, &[], 1), None);
    }

    #[test]
    fn the_pool_the_pick_draws_from_is_bounded() {
        // Two hundred candidates, all worse than the first fifteen. If the pick
        // drew from everything, the tail would show up.
        let current = features(120.0, 0.5);
        let good: Vec<TrackFeatures> = (0..CANDIDATE_POOL_SIZE)
            .map(|_| features(120.0, 0.5))
            .collect();
        let bad: Vec<TrackFeatures> = (0..200).map(|_| features(200.0, 0.0)).collect();

        let pool: Vec<Candidate<'_>> = good
            .iter()
            .chain(bad.iter())
            .map(|track| candidate(None, track))
            .collect();

        let allowed: std::collections::BTreeSet<_> =
            good.iter().map(|track| track.media_file_id).collect();

        for seed in 0..200u64 {
            let chosen = choose_next(Some(&current), &pool, &[], seed * 7_919).expect("a track");
            assert!(
                allowed.contains(&chosen),
                "the pick reached past the top {CANDIDATE_POOL_SIZE}"
            );
        }
    }
}
