//! Background analysis work items.
//!
//! Not one of the named domain types, but the schema defines the
//! `analysis_jobs` table and the layering requires an
//! `AnalysisJobRepositoryPort`, so the entity those two refer to needs a home.

use super::ids::{AnalysisJobId, MediaFileId};
use super::value_objects::Timestamp;
use crate::{CoreError, Result};

/// How many times a failing job is retried before it is parked.
///
/// A calibration knob: transient failures (a file locked by another program)
/// deserve a retry, a corrupt file does not deserve an infinite loop.
pub const MAX_ATTEMPTS: u8 = 3;

/// What a job is supposed to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisKind {
    /// Read tags and artwork.
    Metadata,
    /// Compute the content hash used for duplicate detection.
    Hash,
    /// Extract DSP audio features.
    Features,
}

impl AnalysisKind {
    /// The text form stored in `analysis_jobs.kind`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Hash => "hash",
            Self::Features => "features",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "metadata" => Ok(Self::Metadata),
            "hash" => Ok(Self::Hash),
            "features" => Ok(Self::Features),
            other => Err(CoreError::invalid(
                "analysis kind",
                format!("unknown kind {other:?}"),
            )),
        }
    }
}

/// Where a job is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnalysisState {
    /// Waiting to be picked up.
    #[default]
    Queued,
    /// Currently being worked on.
    Running,
    /// Finished successfully.
    Done,
    /// Failed and out of retries.
    Failed,
}

impl AnalysisState {
    /// The text form stored in `analysis_jobs.state`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(CoreError::invalid(
                "analysis state",
                format!("unknown state {other:?}"),
            )),
        }
    }

    /// True once the job needs no further work.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

/// One unit of background work against one media file.
///
/// Global rather than per-profile: the results describe the file, so two profiles
/// sharing a file share its analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisJob {
    /// Stable identifier.
    pub id: AnalysisJobId,
    /// File to analyse.
    pub media_file_id: MediaFileId,
    /// What to produce.
    pub kind: AnalysisKind,
    /// Lifecycle state.
    pub state: AnalysisState,
    /// Higher runs first. Metadata beats hashing beats feature extraction,
    /// because the library is unusable without titles but merely less clever
    /// without BPM.
    pub priority: i32,
    /// How many times this job has been attempted.
    pub attempts: u8,
    /// Message from the last failure.
    pub error: Option<String>,
    /// When the job was enqueued.
    pub created_at: Timestamp,
    /// When the job last changed state.
    pub updated_at: Timestamp,
}

impl AnalysisJob {
    /// True when a failed job still has retries left.
    pub const fn can_retry(&self) -> bool {
        self.attempts < MAX_ATTEMPTS
    }
}

#[cfg(test)]
mod tests {
    use super::{AnalysisJob, AnalysisKind, AnalysisState, MAX_ATTEMPTS};
    use crate::domain::ids::{AnalysisJobId, MediaFileId};
    use crate::domain::value_objects::Timestamp;

    fn job(attempts: u8) -> AnalysisJob {
        AnalysisJob {
            id: AnalysisJobId::new(),
            media_file_id: MediaFileId::new(),
            kind: AnalysisKind::Features,
            state: AnalysisState::Queued,
            priority: 0,
            attempts,
            error: None,
            created_at: Timestamp::from_millis(1_754_611_200_000),
            updated_at: Timestamp::from_millis(1_754_611_200_000),
        }
    }

    #[test]
    fn retries_are_bounded() {
        assert!(job(0).can_retry());
        assert!(job(MAX_ATTEMPTS - 1).can_retry());
        assert!(!job(MAX_ATTEMPTS).can_retry());
    }

    #[test]
    fn only_finished_states_are_terminal() {
        assert!(!AnalysisState::Queued.is_terminal());
        assert!(!AnalysisState::Running.is_terminal());
        assert!(AnalysisState::Done.is_terminal());
        assert!(AnalysisState::Failed.is_terminal());
    }
}
