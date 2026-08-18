//! What the listener has been listening to, and forgetting it on time.
//!
//! Two duties that look unrelated and are the same one. History is kept for
//! thirty days (PROJECT_MASTER 2.6), so every number here is a number about a
//! window — and the window is enforced by deleting what falls out of it, not by
//! filtering it out of a query. Data that is only hidden is data that is still
//! there.

use std::sync::Arc;

use crate::Result;
use crate::application::context::AppContext;
use crate::domain::ids::ProfileId;
use crate::domain::policies::retention_policy::cutoff;
use crate::domain::ports::repositories::{
    PlayEventRepositoryPort, StatsRepositoryPort, TrackRepositoryPort,
};
use crate::domain::profile::HISTORY_RETENTION_DAYS;
use crate::domain::stats::ListeningSummary;
use crate::domain::track::TrackSummary;
use crate::domain::value_objects::Timestamp;

/// How many tracks the dashboard names.
///
/// A calibration knob. Long enough to be a picture of a month, short enough to
/// be read rather than scrolled.
pub const TOP_TRACKS: u32 = 10;

/// Everything statistics talk to.
pub struct StatsPorts {
    /// The events themselves, for writing off what has expired.
    pub history: Arc<dyn PlayEventRepositoryPort>,
    /// The counting done over them.
    pub stats: Arc<dyn StatsRepositoryPort>,
    /// The library, for turning identifiers back into titles.
    pub tracks: Arc<dyn TrackRepositoryPort>,
}

/// A month of listening, ready to be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct ListeningReport {
    /// How many days it covers.
    pub days: u16,
    /// The totals.
    pub summary: ListeningSummary,
    /// What was played through most, with how many times.
    pub top: Vec<(TrackSummary, u32)>,
    /// Whether this listener is keeping history at all.
    ///
    /// An empty report means two very different things — nothing played, or
    /// nothing recorded — and a screen that cannot tell them apart will show
    /// somebody an empty page and let them wonder.
    pub keeping: bool,
}

/// Listening statistics and the retention that bounds them.
pub struct StatsService {
    context: Arc<AppContext>,
    ports: StatsPorts,
}

impl StatsService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: StatsPorts) -> Self {
        Self { context, ports }
    }

    /// What the active profile has listened to inside the retention window.
    pub fn report(&self) -> Result<ListeningReport> {
        let profile_id = self.context.require_active_profile()?;
        let keeping = self
            .context
            .profiles
            .get(profile_id)?
            .is_some_and(|profile| profile.history_enabled);

        let since = self.window_start();
        let summary = self.ports.stats.summary(profile_id, since)?;

        // Resolved against the library rather than the catalogue: a track
        // removed from this profile's library is no longer something to show
        // them a count for, however often they played it last month.
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;
        let top = self
            .ports
            .stats
            .top_tracks(profile_id, since, TOP_TRACKS)?
            .into_iter()
            .filter_map(|(media_file_id, plays)| {
                library
                    .iter()
                    .find(|summary| summary.media_file_id == media_file_id)
                    .map(|summary| (summary.clone(), plays))
            })
            .collect();

        Ok(ListeningReport {
            days: HISTORY_RETENTION_DAYS,
            summary,
            top,
            keeping,
        })
    }

    /// Deletes what has aged out of every profile's window.
    ///
    /// Every profile, not the active one: the retention promise is about data
    /// on the disk, and a profile nobody has opened for a month is exactly the
    /// one whose history nobody has cleared.
    ///
    /// Returns how many events went, so a caller can say so.
    pub fn purge_expired(&self) -> Result<u64> {
        let now = self.context.now();
        let mut removed = 0;

        for profile in self.context.profiles.list()? {
            let cutoff = cutoff(now, profile.history_retention_days);
            removed += self.ports.history.purge_before(profile.id, cutoff)?;
        }
        Ok(removed)
    }

    /// Forgets everything a profile has listened to.
    ///
    /// What turning history off means: not "stop writing", but "there is
    /// nothing written". Keeping the old rows would leave a month of listening
    /// behind a switch that says it is off (PROJECT_MASTER 1.4).
    pub fn forget(&self, profile_id: ProfileId) -> Result<u64> {
        self.ports.history.purge_all(profile_id)
    }

    /// The oldest moment the report may look at.
    fn window_start(&self) -> Timestamp {
        cutoff(self.context.now(), HISTORY_RETENTION_DAYS)
    }
}
