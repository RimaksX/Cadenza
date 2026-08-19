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
    /// Chosen by smart radio, and by this station.
    ///
    /// The station rides on the source rather than beside it. Section 7.4 has
    /// them as two columns with a `CHECK` holding them together — "a session id
    /// only where the source is radio" — and a rule a database has to be told is
    /// a rule the type can simply not allow to be broken (MASTER_ISSUES 61).
    Radio(RadioSessionId),
    /// Explicitly queued by the listener.
    Manual,
}

impl PlaySource {
    /// The text form stored in `play_events.source`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Playlist => "playlist",
            Self::Radio(_) => "radio",
            Self::Manual => "manual",
        }
    }

    /// The station responsible, where one was.
    ///
    /// What goes in `play_events.radio_session_id`, and the only thing that
    /// makes "how much of this station did I listen to" answerable.
    pub const fn radio_session(self) -> Option<RadioSessionId> {
        match self {
            Self::Radio(session_id) => Some(session_id),
            _ => None,
        }
    }

    /// Parses the stored pair: the text form and the session column beside it.
    ///
    /// Both, because either alone can describe a row the schema forbids. A
    /// radio listen with no station and a library listen with one are equally
    /// impossible, and this is where the reading of them stops.
    pub fn parse(text: &str, session_id: Option<RadioSessionId>) -> Result<Self> {
        match (text, session_id) {
            ("radio", Some(session_id)) => Ok(Self::Radio(session_id)),
            ("radio", None) => Err(CoreError::invalid(
                "play source",
                "a radio listen with no station",
            )),
            (_, Some(_)) => Err(CoreError::invalid(
                "play source",
                format!("a {text} listen cannot belong to a station"),
            )),
            ("library", None) => Ok(Self::Library),
            ("playlist", None) => Ok(Self::Playlist),
            ("manual", None) => Ok(Self::Manual),
            (other, None) => Err(CoreError::invalid(
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

/// What a profile listened to over a window of days.
///
/// Counted by the database rather than assembled from events: a month of
/// listening is thousands of rows, and reading them all to add them up is a
/// query pretending to be a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListeningSummary {
    /// Listens that began, however they ended.
    pub started: u32,
    /// Listens that got past the halfway mark.
    pub completed: u32,
    /// Listens abandoned inside the skip window.
    pub skipped: u32,
    /// Distinct files heard.
    pub tracks: u32,
    /// Audio actually heard, summed.
    pub listened: DurationMs,
}

impl ListeningSummary {
    /// The share of listens that were abandoned early, `0.0..=1.0`.
    ///
    /// Zero for a listener who has played nothing: no starts means no skips,
    /// and a rate of nothing over nothing is not one.
    pub fn skip_rate(&self) -> f32 {
        if self.started == 0 {
            return 0.0;
        }
        self.skipped as f32 / self.started as f32
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
    /// What put it on the queue, and — for radio — which station.
    pub source: PlaySource,
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
    use crate::domain::ids::RadioSessionId;

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
        let station = RadioSessionId::new();
        for source in [
            PlaySource::Library,
            PlaySource::Playlist,
            PlaySource::Radio(station),
            PlaySource::Manual,
        ] {
            assert_eq!(
                PlaySource::parse(source.as_str(), source.radio_session()).expect("round trip"),
                source
            );
        }
        assert!(PlaySource::parse("import", None).is_err());
    }

    #[test]
    fn the_pair_the_schema_forbids_is_refused() {
        let station = RadioSessionId::new();
        assert!(
            PlaySource::parse("radio", None).is_err(),
            "a station is what makes a radio listen a radio listen"
        );
        assert!(
            PlaySource::parse("library", Some(station)).is_err(),
            "nothing else can belong to a station"
        );
    }
}
