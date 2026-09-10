//! What the interface reads.
//!
//! The application layer publishes immutable snapshots and the UI renders them
//! . Nothing here has behaviour beyond arithmetic on its own fields: a view
//! state that can decide something is a business rule that escaped into the
//! layer least able to test it.
//!
//! Listings are [`crate::domain::track::TrackSummary`], which is a read model
//! rather than a view: the same rows feed the queue, playlists and radio, none
//! of which are the interface.

use crate::domain::playback::PlaybackState;
use crate::domain::queue::RepeatMode;
use crate::domain::track::TrackSummary;
use crate::domain::value_objects::{DurationMs, PlaybackPosition, Volume};

/// Everything the player bar draws.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    /// Whether audio is running.
    pub state: PlaybackState,
    /// The loaded track, if any.
    pub track: Option<TrackSummary>,
    /// How far playback has reached.
    pub position: PlaybackPosition,
    /// Length of the loaded track.
    pub duration: DurationMs,
    /// The level the slider shows.
    pub volume: Volume,
    /// Whether output is silenced without losing the level.
    pub muted: bool,
}

impl Default for PlayerView {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            track: None,
            position: PlaybackPosition::START,
            duration: DurationMs::ZERO,
            volume: Volume::default(),
            muted: false,
        }
    }
}

impl PlayerView {
    /// Progress through the track, `0.0..=1.0`.
    ///
    /// Zero for a track of unknown length rather than a division by zero: a
    /// progress bar that jumps to full because a file's header lied is worse
    /// than one that never moves.
    pub fn progress(&self) -> f32 {
        self.position.fraction_of(self.duration)
    }

    /// How much of the track is left.
    pub fn remaining(&self) -> DurationMs {
        self.duration.saturating_sub(self.position.elapsed())
    }
}

/// What the transport buttons around the play button need.
///
/// Counts and flags rather than rows: this is read on every tick, and the
/// listing the queue panel draws is asked for separately when it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueView {
    /// What happens at the end of the track.
    pub repeat: RepeatMode,
    /// Whether the continuation is shuffled.
    pub shuffle: bool,
    /// How many tracks are waiting, excluding the current one.
    pub pending: usize,
    /// Whether anything played before the current track.
    pub has_previous: bool,
    /// Whether anything follows it.
    pub has_next: bool,
}

impl QueueView {
    /// True when repeat is on in either mode — what the button's lit state
    /// shows, with the mode itself distinguishing the two.
    pub const fn repeats(&self) -> bool {
        !matches!(self.repeat, RepeatMode::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::PlayerView;
    use crate::domain::value_objects::{DurationMs, PlaybackPosition};

    #[test]
    fn progress_is_zero_when_the_length_is_unknown() {
        let view = PlayerView {
            position: PlaybackPosition::from_secs(30),
            duration: DurationMs::ZERO,
            ..PlayerView::default()
        };
        assert_eq!(view.progress(), 0.0, "not a division by zero");
        assert_eq!(view.remaining(), DurationMs::ZERO);
    }

    #[test]
    fn progress_is_the_fraction_played() {
        let view = PlayerView {
            position: PlaybackPosition::from_secs(60),
            duration: DurationMs::from_secs(240),
            ..PlayerView::default()
        };
        assert!((view.progress() - 0.25).abs() < f32::EPSILON);
        assert_eq!(view.remaining(), DurationMs::from_secs(180));
    }
}
