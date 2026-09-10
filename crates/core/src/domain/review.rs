//! The import review queue.
//!
//! Cadenza never silently discards a file or silently merges a duplicate. Anything
//! ambiguous lands here and waits for the listener to decide.

use super::ids::{ImportReviewId, MediaFileId, ProfileId};
use super::value_objects::Timestamp;
use crate::{CoreError, Result};

/// Why a file needs a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewReason {
    /// The same content already exists at another path.
    Duplicate,
    /// Tags could not be read.
    UnreadableMetadata,
    /// The extension is supported but the stream would not decode.
    UndecodableAudio,
    /// The file vanished between being seen and being imported.
    MissingFile,
}

impl ReviewReason {
    /// The text form stored in `import_review.reason`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duplicate => "duplicate",
            Self::UnreadableMetadata => "unreadable_metadata",
            Self::UndecodableAudio => "undecodable_audio",
            Self::MissingFile => "missing_file",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "duplicate" => Ok(Self::Duplicate),
            "unreadable_metadata" => Ok(Self::UnreadableMetadata),
            "undecodable_audio" => Ok(Self::UndecodableAudio),
            "missing_file" => Ok(Self::MissingFile),
            other => Err(CoreError::invalid(
                "review reason",
                format!("unknown reason {other:?}"),
            )),
        }
    }
}

/// Where an entry is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReviewState {
    /// Awaiting a decision.
    #[default]
    Pending,
    /// The listener chose an action and it was applied.
    Resolved,
    /// The listener chose to ignore it.
    Dismissed,
}

impl ReviewState {
    /// The text form stored in `import_review.state`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "pending" => Ok(Self::Pending),
            "resolved" => Ok(Self::Resolved),
            "dismissed" => Ok(Self::Dismissed),
            other => Err(CoreError::invalid(
                "review state",
                format!("unknown state {other:?}"),
            )),
        }
    }

    /// True when the entry still needs the listener's attention.
    pub const fn needs_attention(self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// What the listener chose to do about a duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewResolution {
    /// Keep what is already in the library and ignore the new file.
    KeepExisting,
    /// Add the new file as a second copy.
    AddAnyway,
    /// Drop the existing entry from the library and take the new file.
    ///
    /// Removes it from the library only. The file on disk is never deleted.
    RemoveExisting,
    /// Keep both and correct the metadata by hand.
    EditMetadata,
}

/// One file awaiting a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReview {
    /// Stable identifier.
    pub id: ImportReviewId,
    /// Owning profile: an import decision is that listener's business alone.
    pub profile_id: ProfileId,
    /// The file that triggered the entry.
    pub media_file_id: MediaFileId,
    /// The file it duplicates, when [`ReviewReason::Duplicate`].
    pub duplicate_media_file_id: Option<MediaFileId>,
    /// Why it needs a decision.
    pub reason: ReviewReason,
    /// Lifecycle state.
    pub state: ReviewState,
    /// When the entry was raised.
    pub created_at: Timestamp,
    /// When it was resolved or dismissed.
    pub resolved_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::{ReviewReason, ReviewState};

    #[test]
    fn only_pending_entries_need_attention() {
        assert!(ReviewState::Pending.needs_attention());
        assert!(!ReviewState::Resolved.needs_attention());
        assert!(!ReviewState::Dismissed.needs_attention());
    }

    #[test]
    fn reason_text_form_round_trips() {
        for reason in [
            ReviewReason::Duplicate,
            ReviewReason::UnreadableMetadata,
            ReviewReason::UndecodableAudio,
            ReviewReason::MissingFile,
        ] {
            assert_eq!(
                ReviewReason::parse(reason.as_str()).expect("round trip"),
                reason
            );
        }
    }
}
