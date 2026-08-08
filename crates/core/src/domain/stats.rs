//! Listening events.
//!
//! Only the raw event lives here. The daily aggregates of PROJECT_MASTER 7.4
//! arrive in M14 together with the local-date handling they need — bucketing a
//! timestamp into "which day was that for this listener" is calendar work, and
//! inventing a date type before there is a consumer for it would be guesswork.

use super::ids::{MediaFileId, PlayEventId, ProfileId, RadioSessionId};
use super::value_objects::{DurationMs, Timestamp};
use crate::{CoreError, Result};

/// What put the track on the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaySource {
    /// Played from the library listing.
    Library,
    /// Played as part of a playlist.
    Playlist,
    /// Chosen by smart radio.
    Radio,
    /// Explicitly queued by the listener.
    Manual,
}

impl PlaySource {
    /// The text form stored in `play_events.source`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Playlist => "playlist",
            Self::Radio => "radio",
            Self::Manual => "manual",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "library" => Ok(Self::Library),
            "playlist" => Ok(Self::Playlist),
            "radio" => Ok(Self::Radio),
            "manual" => Ok(Self::Manual),
            other => Err(CoreError::invalid(
                "play source",
                format!("unknown source {other:?}"),
            )),
        }
    }
}

/// How a listen ended (PROJECT_MASTER 2.6).
///
/// One enum rather than the schema's two independent `completed` and `skipped`
/// flags, which between them can express "completed and skipped" — a state the
/// rules do not allow. The repository maps this to the two columns on the way
/// out and validates on the way in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayOutcome {
    /// More than half the track was played.
    Completed,
    /// Switched away within the first few seconds.
    Skipped,
    /// Neither: switched away after the skip window but before the halfway mark.
    Partial,
}

impl PlayOutcome {
    /// True when this counts towards play counts and top-track rankings.
    pub const fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }

    /// True when this counts towards skip rates and negative radio feedback.
    pub const fn is_skipped(self) -> bool {
        matches!(self, Self::Skipped)
    }
}

/// One listen, recorded only when the profile has history enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayEvent {
    /// Stable identifier.
    pub id: PlayEventId,
    /// Owning profile.
    pub profile_id: ProfileId,
    /// What was played.
    pub media_file_id: MediaFileId,
    /// What put it on the queue.
    pub source: PlaySource,
    /// The radio session responsible, when [`PlaySource::Radio`].
    pub radio_session_id: Option<RadioSessionId>,
    /// When playback of this track began.
    pub started_at: Timestamp,
    /// When it ended. Absent while still playing.
    pub ended_at: Option<Timestamp>,
    /// How much audio was actually heard, excluding paused time.
    pub played: DurationMs,
    /// Track length at the time of the listen.
    ///
    /// Copied rather than joined: the file may later be removed or replaced, and
    /// a completion rate computed against a missing file is worthless.
    pub duration: DurationMs,
    /// How the listen ended.
    pub outcome: PlayOutcome,
}

#[cfg(test)]
mod tests {
    use super::{PlayOutcome, PlaySource};

    #[test]
    fn outcomes_are_mutually_exclusive() {
        assert!(PlayOutcome::Completed.is_completed());
        assert!(!PlayOutcome::Completed.is_skipped());
        assert!(PlayOutcome::Skipped.is_skipped());
        assert!(!PlayOutcome::Skipped.is_completed());
        assert!(!PlayOutcome::Partial.is_completed());
        assert!(!PlayOutcome::Partial.is_skipped());
    }

    #[test]
    fn source_text_form_round_trips() {
        for source in [
            PlaySource::Library,
            PlaySource::Playlist,
            PlaySource::Radio,
            PlaySource::Manual,
        ] {
            assert_eq!(
                PlaySource::parse(source.as_str()).expect("round trip"),
                source
            );
        }
        assert!(PlaySource::parse("import").is_err());
    }
}
