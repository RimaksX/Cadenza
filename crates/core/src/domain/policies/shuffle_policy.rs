//! Constraints on what smart shuffle is allowed to play next.
//!
//! The hard rules of PROJECT_MASTER 9.2 live here and are enforced from M7's
//! basic shuffle onwards. The scoring and weighted selection of 9.4 arrive in
//! M12 and reuse [`super::transition_policy::transition_score`].

use crate::domain::ids::ArtistId;
use crate::domain::media_file::FileState;

/// How many recently played tracks an artist is barred from reappearing within.
///
/// A calibration knob. Too small and a three-track artist dominates a small
/// library; too large and shuffling a library of one artist stalls, which is why
/// the caller must be prepared to relax the constraint when no candidate passes.
pub const ARTIST_COOLDOWN: usize = 3;

/// How many top-scoring candidates the weighted random pick chooses among.
///
/// PROJECT_MASTER 9.4 says "top 10-20"; this is where in that range it sits.
pub const CANDIDATE_POOL_SIZE: usize = 15;

/// Reorders a pool into a random permutation.
///
/// This is the whole of M7's basic shuffle. A permutation satisfies the first
/// hard rule of 9.2 by construction — every track plays once before any plays
/// twice — and says nothing about which order is *good*, which is what the
/// scoring of 9.4 adds in M12.
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
pub fn artist_on_cooldown(
    candidate: Option<ArtistId>,
    recently_played: &[Option<ArtistId>],
) -> bool {
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
    candidate_artist: Option<ArtistId>,
    recently_played_artists: &[Option<ArtistId>],
) -> bool {
    file_state.is_playable()
        && !already_played
        && !artist_on_cooldown(candidate_artist, recently_played_artists)
}

#[cfg(test)]
mod tests {
    use super::{ARTIST_COOLDOWN, artist_on_cooldown, is_eligible, shuffle};
    use crate::domain::ids::ArtistId;
    use crate::domain::media_file::FileState;

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
        let artist = ArtistId::new();
        let history = vec![Some(ArtistId::new()), Some(artist)];
        assert!(artist_on_cooldown(Some(artist), &history));
    }

    #[test]
    fn the_cooldown_expires_after_enough_other_tracks() {
        let artist = ArtistId::new();
        let mut history = vec![Some(artist)];
        for _ in 0..ARTIST_COOLDOWN {
            history.push(Some(ArtistId::new()));
        }
        assert!(!artist_on_cooldown(Some(artist), &history));
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
        assert!(!artist_on_cooldown(Some(ArtistId::new()), &[]));
    }

    #[test]
    fn every_hard_rule_can_veto_a_candidate() {
        let artist = ArtistId::new();
        let clean_history = vec![Some(ArtistId::new())];

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
}
