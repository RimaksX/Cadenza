//! Transport rules: what "previous" means, and how tracks hand over.

use crate::domain::ids::MediaFileId;
use crate::domain::playback::TransitionProfile;
use crate::domain::queue::{QueueOrigin, RepeatMode};
use crate::domain::settings::PlaybackSettings;
use crate::domain::value_objects::{DurationMs, PlaybackPosition};

/// Past this point, "previous" restarts the current track instead of going back.
///
/// Roughly three seconds; this is the exact threshold that approximation
/// becomes.
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
/// This splits by source rather than by user preference: radio and playlists
/// are continuous material and stay gapless, while ordinary library playback
/// crossfades. The crossfade switch therefore only governs the library case —
/// turning it on does not start crossfading album playthroughs, which is what
/// the rule intends and what listeners expect.
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
/// Order only. What *shuffle* plays next is a different question with different
/// inputs — what has been heard, who made it, what it sounds like — and it is
/// answered by [`super::shuffle_policy::choose_next`].
pub fn next_in_library(
    library: &[MediaFileId],
    current: MediaFileId,
    repeat: RepeatMode,
) -> Option<MediaFileId> {
    let position = library.iter().position(|id| *id == current)?;
    match library.get(position + 1) {
        Some(next) => Some(*next),
        // The end of the list. Repeat all goes back to the top; repeat off is
        // what "stop once the queue is exhausted" means when the queue is a
        // library.
        None if repeat == RepeatMode::All => library.first().copied(),
        None => None,
    }
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
            next_in_library(&library, library[0], RepeatMode::Off),
            Some(library[1])
        );
    }

    #[test]
    fn the_bottom_of_the_list_is_the_end_unless_repeat_says_otherwise() {
        let library = library(3);
        let last = library[2];

        assert_eq!(
            next_in_library(&library, last, RepeatMode::Off),
            None,
            "nothing follows the last row"
        );
        assert_eq!(
            next_in_library(&library, last, RepeatMode::All),
            Some(library[0]),
            "repeat all goes back to the top"
        );
    }

    #[test]
    fn a_track_that_is_no_longer_in_the_library_has_nothing_after_it() {
        let library = library(3);
        assert_eq!(
            next_in_library(&library, MediaFileId::new(), RepeatMode::Off),
            None
        );
    }
}
