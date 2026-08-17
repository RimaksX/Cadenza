//! Radio: an endless stream built out of the listener's own library.
//!
//! Nothing here reaches outside the machine (PROJECT_MASTER 2.6). A station is
//! a mood, a seed track and the same weighted sum every time — the ranking of
//! 10.4 — applied to files that are already on disk and already analysed.
//!
//! The service produces *picks*. It does not play them and does not own a
//! queue: [`super::QueueService`] puts them in the radio lane, where a manual
//! queue still outranks them (10.5). Keeping the two apart is what lets radio
//! be tested without a speaker and the queue be tested without a mood.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::application::context::AppContext;
use crate::domain::ids::{MediaFileId, MoodId, RadioSessionId, RadioSessionItemId};
use crate::domain::mood::MoodPreset;
use crate::domain::policies::radio_policy::{RankingWeights, mood_score, rank};
use crate::domain::policies::shuffle_policy::ARTIST_COOLDOWN;
use crate::domain::policies::transition_policy::transition_score;
use crate::domain::ports::repositories::{
    MoodRepositoryPort, RadioRepositoryPort, TrackFeaturesRepositoryPort, TrackRepositoryPort,
};
use crate::domain::radio::{
    MAX_BATCH_SIZE, MIN_BATCH_SIZE, PickReason, RadioFeedback, RadioSession, RadioSessionItem,
};
use crate::domain::track::{TrackFeatures, TrackSummary};
use crate::domain::value_objects::{DurationMs, Timestamp};
use crate::{CoreError, Result};

/// How far back a station remembers having played something.
///
/// A calibration knob. Long enough that a track offered this morning is not
/// offered again this afternoon, short enough that a library smaller than a
/// week of listening does not run out of fresh material. Measured against
/// radio's own picks rather than listening history, which nothing writes yet
/// (MASTER_ISSUES 49).
const FRESHNESS_WINDOW: DurationMs = DurationMs::from_secs(7 * 24 * 60 * 60);

/// Everything radio talks to.
pub struct RadioPorts {
    /// Sessions, their picks and the verdicts on them.
    pub radio: Arc<dyn RadioRepositoryPort>,
    /// The moods a session can be started in.
    pub moods: Arc<dyn MoodRepositoryPort>,
    /// The library radio draws from.
    pub tracks: Arc<dyn TrackRepositoryPort>,
    /// What that library sounds like.
    pub features: Arc<dyn TrackFeaturesRepositoryPort>,
}

/// A station, and the picks it is making.
pub struct RadioService {
    context: Arc<AppContext>,
    ports: RadioPorts,
    /// The session being listened to, with the mood it was started in.
    ///
    /// Held rather than re-read: every batch needs both, and a mood is a row
    /// that cannot change while a session is running — built-ins are not
    /// editable and a custom one being edited mid-session is a change the
    /// listener will hear at the next station they start.
    live: RwLock<Option<(RadioSession, MoodPreset)>>,
    /// How many picks the session has made, which is an item's position.
    picks: AtomicU32,
}

impl RadioService {
    /// Wires the service to the shared context and its ports.
    pub fn new(context: Arc<AppContext>, ports: RadioPorts) -> Self {
        Self {
            context,
            ports,
            live: RwLock::new(None),
            picks: AtomicU32::new(0),
        }
    }

    /// The moods this listener can start from: the eight built-ins and their own.
    pub fn moods(&self) -> Result<Vec<MoodPreset>> {
        let profile_id = self.context.require_active_profile()?;
        self.ports.moods.list_for_profile(profile_id)
    }

    /// The session being listened to, if any.
    pub fn session(&self) -> Option<RadioSession> {
        self.live
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .map(|(session, _)| session.clone())
    }

    /// The mood the live session is in.
    pub fn mood(&self) -> Option<MoodPreset> {
        self.live
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .map(|(_, mood)| mood.clone())
    }

    /// Starts a station.
    ///
    /// `seed` is what the listener was playing when they asked for radio, and
    /// it is only a starting point for the transition score: the mood decides
    /// what the station *is*.
    pub fn start(&self, mood_id: MoodId, seed: Option<MediaFileId>) -> Result<RadioSession> {
        let profile_id = self.context.require_active_profile()?;
        let mood = self
            .ports
            .moods
            .get(mood_id)?
            .ok_or_else(|| CoreError::not_found("mood", mood_id))?;

        // A mood belonging to somebody else is not a mood this listener has.
        if let Some(owner) = mood.profile_id
            && owner != profile_id
        {
            return Err(CoreError::not_found("mood", mood_id));
        }

        let now = self.context.now();
        let session = RadioSession {
            id: RadioSessionId::new(),
            profile_id,
            mood_id,
            seed_media_file_id: seed,
            params_json: None,
            created_at: now,
            updated_at: now,
        };

        self.ports.radio.save_session(&session)?;
        *self.live.write().unwrap_or_else(|err| err.into_inner()) = Some((session.clone(), mood));
        self.picks.store(0, Ordering::Relaxed);

        Ok(session)
    }

    /// Forgets the live session. The rows stay: they are what was listened to.
    pub fn stop(&self) {
        *self.live.write().unwrap_or_else(|err| err.into_inner()) = None;
    }

    /// Chooses the next `wanted` tracks and records why.
    ///
    /// Between eight and fifteen, which is 10.5's batch. Fewer come back only
    /// when the library has run out of anything the session has not already
    /// offered — a station in a small library ends rather than repeating
    /// itself, and the caller decides what to say about that.
    pub fn next_batch(&self, wanted: usize) -> Result<Vec<MediaFileId>> {
        let Some((session, mood)) = self
            .live
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
        else {
            return Err(CoreError::invalid("radio", "no station is playing"));
        };

        let wanted = wanted.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE);
        let profile_id = session.profile_id;

        let library = self.ports.tracks.summaries_for_profile(profile_id)?;
        let features = self.ports.features.list_all()?;
        let preferences = self.ports.radio.preferences(profile_id)?;
        let recent = self.ports.radio.last_offered(profile_id)?;

        // Everything the session has already offered. A station may not repeat
        // itself while it still has anything else to play — 9.2's first rule,
        // which radio inherits.
        let mut offered: Vec<MediaFileId> = self
            .ports
            .radio
            .recent_items(session.id, u32::MAX)?
            .into_iter()
            .map(|item| item.media_file_id)
            .collect();

        let find = |id: MediaFileId| features.iter().find(|row| row.media_file_id == id);
        let mut previous: Option<&TrackFeatures> = session.seed_media_file_id.and_then(find);
        let mut artists: Vec<Option<&str>> = Vec::new();
        let mut chosen = Vec::new();

        for _ in 0..wanted {
            let Some((summary, reason)) = self.best_candidate(
                &library,
                &offered,
                previous,
                &mood,
                &preferences,
                &recent,
                &artists,
                find,
            ) else {
                break;
            };

            let position = self.picks.fetch_add(1, Ordering::Relaxed);
            self.ports.radio.append_item(&RadioSessionItem {
                id: RadioSessionItemId::new(),
                session_id: session.id,
                media_file_id: summary.media_file_id,
                position,
                reason: Some(reason),
                created_at: self.context.now(),
            })?;

            offered.push(summary.media_file_id);
            artists.push(summary.artist.as_deref());
            previous = find(summary.media_file_id);
            chosen.push(summary.media_file_id);
        }

        Ok(chosen)
    }

    /// Records what the listener thought of a pick.
    ///
    /// A skip is a weaker signal than a dislike and both are weaker than a
    /// like is strong; the weights are [`RadioFeedback::weight`]'s. What it
    /// changes is the next batch, and every batch after that in any session
    /// (PROJECT_MASTER 10.5).
    pub fn feedback(&self, media_file_id: MediaFileId, verdict: RadioFeedback) -> Result<()> {
        let Some(session) = self.session() else {
            // Not an error: a listener pressing skip during ordinary playback
            // is not saying anything about a station they are not listening to.
            return Ok(());
        };

        // Nor is a verdict about a track this station never offered. That
        // happens whenever a listener presses next on something they queued by
        // hand while a station is running: they are skipping their own choice,
        // not the station's.
        let offered = self
            .ports
            .radio
            .recent_items(session.id, u32::MAX)?
            .into_iter()
            .any(|item| item.media_file_id == media_file_id);
        if !offered {
            return Ok(());
        }

        self.ports
            .radio
            .set_feedback(session.id, media_file_id, verdict, self.context.now())
    }

    /// The highest-scoring candidate, and why it won.
    #[allow(clippy::too_many_arguments)]
    fn best_candidate<'a>(
        &self,
        library: &'a [TrackSummary],
        offered: &[MediaFileId],
        previous: Option<&TrackFeatures>,
        mood: &MoodPreset,
        preferences: &[(MediaFileId, f32)],
        recent: &[(MediaFileId, Timestamp)],
        artists: &[Option<&str>],
        find: impl Fn(MediaFileId) -> Option<&'a TrackFeatures>,
    ) -> Option<(&'a TrackSummary, PickReason)> {
        let weights = RankingWeights::DEFAULT;
        let mut best: Option<(f32, &TrackSummary, PickReason)> = None;

        for summary in library {
            if offered.contains(&summary.media_file_id) {
                continue;
            }

            let features = find(summary.media_file_id);
            let reason = PickReason {
                mood: mood_score(&mood.rules, features),
                transition: match (previous, features) {
                    (Some(previous), Some(features)) => transition_score(previous, features),
                    // Nothing to follow, or nothing to follow it with: the term
                    // says neither good nor bad rather than dragging the whole
                    // candidate down.
                    _ => crate::domain::policies::NEUTRAL_SCORE,
                },
                preference: preference_of(summary.media_file_id, preferences),
                freshness: freshness_of(summary.media_file_id, recent, self.context.now()),
                repeats_artist: repeats_artist(summary.artist.as_deref(), artists),
                score: 0.0,
            };

            let score = rank(
                &weights,
                reason.mood,
                reason.transition,
                reason.preference,
                reason.freshness,
                reason.repeats_artist,
                noise(),
            );

            if best.as_ref().is_none_or(|(top, _, _)| score > *top) {
                best = Some((score, summary, PickReason { score, ..reason }));
            }
        }

        best.map(|(_, summary, reason)| (summary, reason))
    }
}

/// What the listener has said about a track, as `0.0..=1.0`.
///
/// Nothing said is half: a track nobody has judged is neither liked nor
/// disliked, and calling it either would make silence an opinion.
fn preference_of(media_file_id: MediaFileId, preferences: &[(MediaFileId, f32)]) -> f32 {
    let total = preferences
        .iter()
        .find(|(id, _)| *id == media_file_id)
        .map_or(0.0, |(_, total)| *total);

    // Two likes reach the top of the scale and two dislikes the bottom. Beyond
    // that the listener is repeating themselves.
    (0.5 + total / 4.0).clamp(0.0, 1.0)
}

/// How long since it was last heard, as `0.0..=1.0`.
fn freshness_of(
    media_file_id: MediaFileId,
    recent: &[(MediaFileId, Timestamp)],
    now: Timestamp,
) -> f32 {
    let Some((_, played_at)) = recent.iter().find(|(id, _)| *id == media_file_id) else {
        // Not in the window at all: as fresh as this can measure.
        return 1.0;
    };

    let ago = now.as_millis().saturating_sub(played_at.as_millis());
    (ago as f32 / FRESHNESS_WINDOW.as_millis() as f32).clamp(0.0, 1.0)
}

/// True when this artist is among the last few the station played.
fn repeats_artist(candidate: Option<&str>, artists: &[Option<&str>]) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    artists
        .iter()
        .rev()
        .take(ARTIST_COOLDOWN)
        .any(|recent| *recent == Some(candidate))
}

/// A number in `0.0..=1.0` for the exploration term.
///
/// From a fresh identifier, which is the only entropy this layer has — the same
/// trade the shuffle makes, and for the same reason.
fn noise() -> f32 {
    ((Uuid::new_v4().as_u128() >> 96) as u32) as f32 / u32::MAX as f32
}
