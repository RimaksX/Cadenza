//! What the player is doing right now.

use super::ids::MediaFileId;
use super::queue::RepeatMode;
use super::value_objects::{DurationMs, PlaybackPosition, Volume};
use crate::{CoreError, Result};

/// How one track gives way to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransitionProfile {
    /// Sample-accurate join with no gap and no overlap.
    ///
    /// The default for radio and playlists, where the material is meant to run
    /// continuously.
    #[default]
    Gapless,
    /// Equal-power overlap between the outgoing and incoming track.
    Crossfade,
}

impl TransitionProfile {
    /// The text form stored in `mood_presets.transition_profile`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gapless => "gapless",
            Self::Crossfade => "crossfade",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "gapless" => Ok(Self::Gapless),
            "crossfade" => Ok(Self::Crossfade),
            other => Err(CoreError::invalid(
                "transition profile",
                format!("unknown profile {other:?}"),
            )),
        }
    }
}

/// Whether audio is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlaybackState {
    /// Nothing loaded, or playback finished.
    #[default]
    Stopped,
    /// Audio is being produced.
    Playing,
    /// A track is loaded and holding its position.
    Paused,
}

impl PlaybackState {
    /// True while audio is being produced.
    pub const fn is_playing(self) -> bool {
        matches!(self, Self::Playing)
    }

    /// True when a track is loaded, whether or not it is running.
    pub const fn has_track(self) -> bool {
        matches!(self, Self::Playing | Self::Paused)
    }
}

/// The track currently loaded, and how far into it playback is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowPlaying {
    /// The file being played.
    pub media_file_id: MediaFileId,
    /// Elapsed position.
    pub position: PlaybackPosition,
    /// Total length of the track.
    pub duration: DurationMs,
}

impl NowPlaying {
    /// Progress through the track, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        self.position.fraction_of(self.duration)
    }

    /// How much of the track is left.
    pub const fn remaining(&self) -> DurationMs {
        self.duration.saturating_sub(self.position.elapsed())
    }
}

/// A complete, self-consistent picture of the player.
///
/// Produced by the application layer and consumed by the UI. The UI holds no
/// playback state of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackSnapshot {
    /// Whether audio is running.
    pub state: PlaybackState,
    /// The loaded track, if any.
    pub now_playing: Option<NowPlaying>,
    /// Output volume.
    pub volume: Volume,
    /// Whether output is muted. Mute is separate from a zero volume so that
    /// unmuting restores the previous level.
    pub muted: bool,
    /// Repeat behaviour.
    pub repeat: RepeatMode,
    /// Whether the automatic continuation is shuffled.
    pub shuffle: bool,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            now_playing: None,
            volume: Volume::default(),
            muted: false,
            repeat: RepeatMode::default(),
            shuffle: false,
        }
    }
}

impl PlaybackSnapshot {
    /// The volume actually sent to the output.
    pub fn effective_volume(&self) -> Volume {
        if self.muted {
            Volume::MUTED
        } else {
            self.volume
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NowPlaying, PlaybackSnapshot, PlaybackState, TransitionProfile};
    use crate::domain::ids::MediaFileId;
    use crate::domain::value_objects::{DurationMs, PlaybackPosition, Volume};

    #[test]
    fn muting_does_not_forget_the_volume() {
        let mut snapshot = PlaybackSnapshot {
            volume: Volume::new(0.4).expect("in range"),
            ..PlaybackSnapshot::default()
        };
        snapshot.muted = true;

        assert_eq!(snapshot.effective_volume(), Volume::MUTED);
        assert_eq!(
            snapshot.volume,
            Volume::new(0.4).expect("in range"),
            "the stored level survives muting"
        );
    }

    #[test]
    fn remaining_time_never_goes_negative() {
        let now_playing = NowPlaying {
            media_file_id: MediaFileId::new(),
            position: PlaybackPosition::from_secs(500),
            duration: DurationMs::from_secs(200),
        };
        assert_eq!(now_playing.remaining(), DurationMs::ZERO);
        assert!((now_playing.progress() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn paused_still_counts_as_having_a_track() {
        assert!(PlaybackState::Paused.has_track());
        assert!(!PlaybackState::Paused.is_playing());
        assert!(!PlaybackState::Stopped.has_track());
    }

    #[test]
    fn transition_text_form_round_trips() {
        for profile in [TransitionProfile::Gapless, TransitionProfile::Crossfade] {
            assert_eq!(
                TransitionProfile::parse(profile.as_str()).expect("round trip"),
                profile
            );
        }
    }
}
