//! Learning what the music sounds like, quietly.
//!
//! This service owns the *bookkeeping* of analysis — which files still need it,
//! which job is next, what happened when one failed — and none of the signal
//! processing, which lives behind [`FeatureExtractorPort`] in the infrastructure
//! layer (PROJECT_MASTER 4.4).
//!
//! Everything here is a single step that returns. Nothing loops, nothing sleeps
//! and nothing spawns: the pacing is the worker's business, and a service that
//! blocked would be a service the interface could not call
//! (PROJECT_MASTER 12.1, "все фоновые задачи должны быть низкоприоритетными и
//! неблокирующими").

use std::sync::Arc;

use crate::application::context::AppContext;
use crate::domain::analysis::{AnalysisJob, AnalysisKind};
use crate::domain::ids::MediaFileId;
use crate::domain::policies::analysis_policy::TOP_UP_BATCH;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::feature_extractor::FeatureExtractorPort;
use crate::domain::ports::repositories::{
    AnalysisJobRepositoryPort, MediaFileRepositoryPort, TrackFeaturesRepositoryPort,
};
use crate::domain::track::TrackFeatures;
use crate::{CoreError, Result};

/// Everything analysis talks to.
pub struct AnalysisPorts {
    /// The work queue.
    pub jobs: Arc<dyn AnalysisJobRepositoryPort>,
    /// Where the answers are kept.
    pub features: Arc<dyn TrackFeaturesRepositoryPort>,
    /// The catalogue, for the path of the file to open.
    pub media_files: Arc<dyn MediaFileRepositoryPort>,
    /// The signal processing itself.
    pub extractor: Arc<dyn FeatureExtractorPort>,
}

/// How far through the library analysis has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisProgress {
    /// Files carrying features from the current extractor.
    pub analysed: u64,
    /// Jobs queued or running.
    pub pending: u64,
}

impl AnalysisProgress {
    /// True when there is nothing left to do.
    pub const fn is_settled(&self) -> bool {
        self.pending == 0
    }
}

/// Background feature extraction, one step at a time.
pub struct AnalysisService {
    context: Arc<AppContext>,
    ports: AnalysisPorts,
}

impl AnalysisService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: AnalysisPorts) -> Self {
        Self { context, ports }
    }

    /// The extractor whose answers count as current.
    pub fn extractor_version(&self) -> &str {
        self.ports.extractor.version()
    }

    /// Queues the next batch of files that still need analysing.
    ///
    /// Returns how many were queued, so a caller can tell "there was nothing to
    /// do" from "there was, and now it is waiting". Idempotent by construction:
    /// a file already queued cannot be queued twice, and a file already
    /// analysed by this extractor is not a candidate.
    pub fn top_up(&self) -> Result<u64> {
        // Nothing is announced: queueing work is not news to anybody, and the
        // event that matters is the one an answer produces.
        self.ports.jobs.enqueue_missing_features(
            self.extractor_version(),
            TOP_UP_BATCH,
            self.context.now(),
        )
    }

    /// Runs the next queued job through to its answer.
    ///
    /// Returns the file analysed, or `None` when there was no work waiting. One
    /// file per call, because this is the unit the worker paces itself by: the
    /// share of the machine analysis is allowed is enforced between calls, not
    /// inside them.
    pub fn run_next(&self) -> Result<Option<MediaFileId>> {
        let Some(job) = self
            .ports
            .jobs
            .claim_next(Some(AnalysisKind::Features), self.context.now())?
        else {
            return Ok(None);
        };

        match self.analyse(&job) {
            Ok(features) => {
                self.ports.features.save(&features)?;
                self.ports.jobs.complete(job.id, self.context.now())?;
                self.context
                    .events
                    .publish(DomainEvent::AnalysisCompleted(job.media_file_id));
                Ok(Some(job.media_file_id))
            }
            // A file that will not analyse is not a reason to stop analysing.
            // The failure is recorded against the job — with its attempt count,
            // which is what stops a corrupt file being retried for ever — and
            // the worker moves on to the next one.
            Err(err) => {
                self.ports
                    .jobs
                    .fail(job.id, &err.to_string(), self.context.now())?;
                Ok(Some(job.media_file_id))
            }
        }
    }

    /// One file's features, if they have been extracted.
    pub fn features(&self, media_file_id: MediaFileId) -> Result<Option<TrackFeatures>> {
        self.ports.features.get(media_file_id)
    }

    /// How far through the library analysis has got.
    pub fn progress(&self) -> Result<AnalysisProgress> {
        Ok(AnalysisProgress {
            analysed: self
                .ports
                .features
                .count_for_extractor(self.extractor_version())?,
            pending: self.ports.jobs.pending_count()?,
        })
    }

    /// Opens the file a job names and extracts its features.
    fn analyse(&self, job: &AnalysisJob) -> Result<TrackFeatures> {
        let media_file = self
            .ports
            .media_files
            .get(job.media_file_id)?
            .ok_or_else(|| CoreError::not_found("media file", job.media_file_id))?;

        if !media_file.state.is_playable() {
            return Err(CoreError::Invalid {
                field: "media file",
                reason: format!(
                    "{} cannot be analysed: it is {}",
                    media_file.path.display(),
                    media_file.state.as_str()
                ),
            });
        }

        self.ports
            .extractor
            .extract(job.media_file_id, &media_file.path)
    }
}
