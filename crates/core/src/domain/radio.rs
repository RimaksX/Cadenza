//! Smart radio sessions.

use super::ids::{MediaFileId, MoodId, ProfileId, RadioSessionId, RadioSessionItemId};
use super::value_objects::Timestamp;

/// Smallest batch the generator produces (PROJECT_MASTER 10.5).
pub const MIN_BATCH_SIZE: usize = 8;

/// Largest batch the generator produces.
pub const MAX_BATCH_SIZE: usize = 15;

/// How few tracks may remain before the batch is topped up.
///
/// A calibration knob. Too low and generation stalls the transition; too high
/// and the generator wastes work on tracks the listener will skip past.
pub const REFILL_THRESHOLD: usize = 3;

// Compile-time invariants. Tuning the knobs above into an inconsistent state
// fails the build rather than a test, because a batch that runs dry before the
// refill triggers would stall playback at the transition.
const _: () = assert!(MIN_BATCH_SIZE <= MAX_BATCH_SIZE);
const _: () = assert!(
    REFILL_THRESHOLD < MIN_BATCH_SIZE,
    "refilling must trigger before the batch runs dry"
);

/// A listener's verdict on a radio pick.
///
/// Feeds back into future generation (PROJECT_MASTER 10.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadioFeedback {
    /// Play more like this.
    Like,
    /// Play less like this.
    Dislike,
    /// Skipped, a weaker negative signal than an explicit dislike.
    Skip,
}

impl RadioFeedback {
    /// How strongly this signal should move preference, signed.
    ///
    /// Calibration knobs. An explicit dislike is a deliberate act and counts for
    /// more than a skip, which may just mean "not right now".
    pub const fn weight(self) -> f32 {
        match self {
            Self::Like => 1.0,
            Self::Dislike => -1.0,
            Self::Skip => -0.3,
        }
    }
}

/// One continuous radio listening session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioSession {
    /// Stable identifier.
    pub id: RadioSessionId,
    /// Owning profile.
    pub profile_id: ProfileId,
    /// The mood or activity that shapes selection.
    pub mood_id: MoodId,
    /// Track the session was seeded from, if the listener started from one.
    pub seed_media_file_id: Option<MediaFileId>,
    /// Serialised generation parameters, opaque until M13 fixes the shape.
    pub params_json: Option<String>,
    /// When the session started.
    pub created_at: Timestamp,
    /// When it last produced a track.
    pub updated_at: Timestamp,
}

/// One track the radio chose, with the reasoning behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioSessionItem {
    /// Stable identifier.
    pub id: RadioSessionItemId,
    /// Owning session.
    pub session_id: RadioSessionId,
    /// The chosen file.
    pub media_file_id: MediaFileId,
    /// Position within the session.
    pub position: u32,
    /// Why this track was picked.
    ///
    /// Kept because the selection is a weighted formula rather than a model: a
    /// pick can always be explained, and that is worth preserving for debugging
    /// and for showing the listener (PROJECT_MASTER ADR 0004).
    pub reason_json: Option<String>,
    /// When it was queued.
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::RadioFeedback;

    #[test]
    fn an_explicit_dislike_outweighs_a_skip() {
        assert!(RadioFeedback::Dislike.weight() < RadioFeedback::Skip.weight());
        assert!(RadioFeedback::Skip.weight() < 0.0);
        assert!(RadioFeedback::Like.weight() > 0.0);
    }
}
