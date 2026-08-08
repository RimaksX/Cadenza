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
    use super::{ARTIST_COOLDOWN, artist_on_cooldown, is_eligible};
    use crate::domain::ids::ArtistId;
    use crate::domain::media_file::FileState;

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
