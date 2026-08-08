//! Transport rules: what "previous" means, and how tracks hand over.

use crate::domain::playback::TransitionProfile;
use crate::domain::queue::QueueOrigin;
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

#[cfg(test)]
mod tests {
    use super::{PREVIOUS_RESTART_THRESHOLD, PreviousAction, previous_action, transition_for};
    use crate::domain::ids::{PlaylistId, RadioSessionId};
    use crate::domain::playback::TransitionProfile;
    use crate::domain::queue::QueueOrigin;
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
}
