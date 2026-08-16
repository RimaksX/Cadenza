//! Transport rules: what "previous" means, and how tracks hand over.

use crate::domain::ids::MediaFileId;
use crate::domain::playback::TransitionProfile;
use crate::domain::queue::{QueueOrigin, RepeatMode};
use crate::domain::settings::PlaybackSettings;
use crate::domain::value_objects::{DurationMs, PlaybackPosition};

/// Past this point, "previous" restarts the current track instead of going back.
///
/// PROJECT_MASTER 2.3 gives "~3 seconds"; this is the exact threshold that
/// approximation becomes.
pub const PREVIOUS_RESTART_THRESHOLD: DurationMs = DurationMs::from_secs(3);

/// What pressing "previous" should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousAction {
    /// Seek back to the start of the track that is already playing.
    RestartCurrent,
    /// Move to the previously played track.
    PreviousTrack,
}

/// Decides what "previous" means at the current position.
pub fn previous_action(position: PlaybackPosition) -> PreviousAction {
    if position.elapsed() >= PREVIOUS_RESTART_THRESHOLD {
        PreviousAction::RestartCurrent
    } else {
        PreviousAction::PreviousTrack
    }
}

/// How the track from `origin` should hand over to the next one.
///
/// PROJECT_MASTER 2.4 splits this by source rather than by user preference:
/// radio and playlists are continuous material and stay gapless, while ordinary
/// library playback crossfades. The crossfade switch therefore only governs the
/// library case — turning it on does not start crossfading album playthroughs,
/// which is what the rule intends and what listeners expect.
pub const fn transition_for(origin: QueueOrigin, settings: &PlaybackSettings) -> TransitionProfile {
    match origin {
        QueueOrigin::Playlist(_) | QueueOrigin::Radio(_) => TransitionProfile::Gapless,
        QueueOrigin::Library => {
            if settings.crossfade_enabled {
                TransitionProfile::Crossfade
            } else {
                TransitionProfile::Gapless
            }
        }
    }
}

/// What plays after `current` when the queue has nothing waiting.
///
/// The library is the continuation of library playback, and it is a
/// continuation nobody has to queue: the rows are already in an order, and the
/// queue is for the tracks a listener chose to hear next rather than the ones
/// that merely come after. `None` means the round is over — the bottom of the
/// list with repeat off, or a library that has nothing else to offer.
///
/// `played` is what has already been heard, which is what stops shuffle
/// playing a track twice before the rest have had a turn (PROJECT_MASTER 9.2's
/// first hard rule). In library order it says nothing: the order is the order.
/// The other two hard rules — the artist cooldown and unplayable files — arrive
/// with the scoring of M12; a listing already excludes what is not in the
/// library.
pub fn next_in_library(
    library: &[MediaFileId],
    current: MediaFileId,
    played: &[MediaFileId],
    shuffle: bool,
    repeat: RepeatMode,
    seed: u64,
) -> Option<MediaFileId> {
    if shuffle {
        return next_at_random(library, current, played, repeat, seed);
    }

    let position = library.iter().position(|id| *id == current)?;
    match library.get(position + 1) {
        Some(next) => Some(*next),
        // The end of the list. Repeat all goes back to the top; repeat off is
        // what "stop once the queue is exhausted" means when the queue is a
        // library (PROJECT_MASTER 2.3).
        None if repeat == RepeatMode::All => library.first().copied(),
        None => None,
    }
}

/// One track drawn from what has not been heard yet.
fn next_at_random(
    library: &[MediaFileId],
    current: MediaFileId,
    played: &[MediaFileId],
    repeat: RepeatMode,
    seed: u64,
) -> Option<MediaFileId> {
    let mut pool: Vec<MediaFileId> = library
        .iter()
        .copied()
        .filter(|id| *id != current && !played.contains(id))
        .collect();

    if pool.is_empty() {
        // Everything has had its turn: repeat all begins the round again, and
        // repeat off stops where ordered playback would stop.
        if repeat != RepeatMode::All {
            return None;
        }
        pool = library
            .iter()
            .copied()
            .filter(|id| *id != current)
            .collect();
        // A library of one is still a library, and repeat all still repeats it.
        if pool.is_empty() {
            return Some(current);
        }
    }

    // The modulo is biased by about one part in 2^58 for any library a listener
    // will ever have, which is not a musical problem — the same trade
    // `shuffle_policy::shuffle` makes.
    pool.get((seed % pool.len() as u64) as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::{
        PREVIOUS_RESTART_THRESHOLD, PreviousAction, next_in_library, previous_action,
        transition_for,
    };
    use crate::domain::ids::{MediaFileId, PlaylistId, RadioSessionId};
    use crate::domain::playback::TransitionProfile;
    use crate::domain::queue::{QueueOrigin, RepeatMode};
    use crate::domain::settings::PlaybackSettings;
    use crate::domain::value_objects::{DurationMs, PlaybackPosition};

    #[test]
    fn early_in_a_track_previous_goes_back() {
        assert_eq!(
            previous_action(PlaybackPosition::from_secs(1)),
            PreviousAction::PreviousTrack
        );
        assert_eq!(
            previous_action(PlaybackPosition::START),
            PreviousAction::PreviousTrack
        );
    }

    #[test]
    fn later_in_a_track_previous_restarts_it() {
        assert_eq!(
            previous_action(PlaybackPosition::from_secs(30)),
            PreviousAction::RestartCurrent
        );
    }

    #[test]
    fn the_threshold_itself_restarts() {
        let at_threshold = PlaybackPosition::from_millis(PREVIOUS_RESTART_THRESHOLD.as_millis());
        let just_under = at_threshold.saturating_sub(DurationMs::from_millis(1));

        assert_eq!(
            previous_action(at_threshold),
            PreviousAction::RestartCurrent
        );
        assert_eq!(previous_action(just_under), PreviousAction::PreviousTrack);
    }

    #[test]
    fn playlists_and_radio_stay_gapless_whatever_the_setting_says() {
        let crossfading = PlaybackSettings {
            crossfade_enabled: true,
            ..PlaybackSettings::default()
        };

        assert_eq!(
            transition_for(QueueOrigin::Playlist(PlaylistId::new()), &crossfading),
            TransitionProfile::Gapless
        );
        assert_eq!(
            transition_for(QueueOrigin::Radio(RadioSessionId::new()), &crossfading),
            TransitionProfile::Gapless
        );
    }

    #[test]
    fn the_crossfade_switch_governs_library_playback_only() {
        let off = PlaybackSettings::default();
        let on = PlaybackSettings {
            crossfade_enabled: true,
            ..PlaybackSettings::default()
        };

        assert_eq!(
            transition_for(QueueOrigin::Library, &off),
            TransitionProfile::Gapless
        );
        assert_eq!(
            transition_for(QueueOrigin::Library, &on),
            TransitionProfile::Crossfade
        );
    }

    fn library(size: usize) -> Vec<MediaFileId> {
        (0..size).map(|_| MediaFileId::new()).collect()
    }

    #[test]
    fn the_library_carries_on_from_the_row_that_is_playing() {
        let library = library(3);
        assert_eq!(
            next_in_library(&library, library[0], &[], false, RepeatMode::Off, 1),
            Some(library[1])
        );
    }

    #[test]
    fn the_bottom_of_the_list_is_the_end_unless_repeat_says_otherwise() {
        let library = library(3);
        let last = library[2];

        assert_eq!(
            next_in_library(&library, last, &[], false, RepeatMode::Off, 1),
            None,
            "nothing follows the last row"
        );
        assert_eq!(
            next_in_library(&library, last, &[], false, RepeatMode::All, 1),
            Some(library[0]),
            "repeat all goes back to the top"
        );
    }

    #[test]
    fn a_track_that_is_no_longer_in_the_library_has_nothing_after_it() {
        let library = library(3);
        assert_eq!(
            next_in_library(&library, MediaFileId::new(), &[], false, RepeatMode::Off, 7),
            None
        );
    }

    #[test]
    fn shuffle_gives_every_track_a_turn_before_any_gets_a_second() {
        let library = library(8);
        let mut played = vec![library[0]];
        let mut current = library[0];

        for step in 1..8u64 {
            let next = next_in_library(
                &library,
                current,
                &played,
                true,
                RepeatMode::Off,
                step.wrapping_mul(0x9e37_79b9_7f4a_7c15),
            )
            .expect("the library has more to offer");

            assert!(!played.contains(&next), "nothing played twice");
            played.push(next);
            current = next;
        }

        assert_eq!(played.len(), 8, "the whole library had a turn");
        assert_eq!(
            next_in_library(&library, current, &played, true, RepeatMode::Off, 3),
            None,
            "and then it stops"
        );
    }

    #[test]
    fn an_exhausted_shuffle_starts_again_under_repeat_all() {
        let library = library(4);
        let played = library.clone();
        let next = next_in_library(&library, library[3], &played, true, RepeatMode::All, 11)
            .expect("the round begins again");

        assert!(library.contains(&next));
        assert_ne!(next, library[3], "but not the track that is playing");
    }

    #[test]
    fn a_library_of_one_repeats_itself_when_asked_to() {
        let library = library(1);
        assert_eq!(
            next_in_library(&library, library[0], &library, true, RepeatMode::All, 5),
            Some(library[0])
        );
        assert_eq!(
            next_in_library(&library, library[0], &library, true, RepeatMode::Off, 5),
            None,
            "and stops when it is not"
        );
    }
}
