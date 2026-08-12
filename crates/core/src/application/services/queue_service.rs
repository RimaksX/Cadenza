//! What plays next.
//!
//! The queue is the only thing that starts a track once the listener has
//! chosen the first one. [`super::PlaybackService`] knows how to play a file
//! and nothing about order; this service owns the order and asks it to play
//! (PROJECT_MASTER 2.3).
//!
//! It sits on top of playback rather than beside it because "next" is one
//! action, not two: deciding what follows and starting it cannot be split
//! across the boundary without putting half the rule in the interface, which is
//! the layer least allowed to hold one.

use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::application::context::AppContext;
use crate::application::view_state::QueueView;
use crate::domain::ids::MediaFileId;
use crate::domain::policies::playback_policy::{PreviousAction, previous_action};
use crate::domain::policies::shuffle_policy;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::repositories::{QueueRepositoryPort, TrackRepositoryPort};
use crate::domain::queue::{Queue, QueueEntry, QueueOrigin, RepeatMode};
use crate::domain::track::TrackSummary;
use crate::domain::value_objects::PlaybackPosition;
use crate::{CoreError, Result};

use super::PlaybackService;

/// Everything the queue talks to.
pub struct QueuePorts {
    /// Where the queue is kept between runs.
    pub queue: Arc<dyn QueueRepositoryPort>,
    /// The library, which is the pool the continuation is built from.
    pub tracks: Arc<dyn TrackRepositoryPort>,
}

/// The playback queue and the transport commands that move through it.
pub struct QueueService {
    context: Arc<AppContext>,
    playback: Arc<PlaybackService>,
    ports: QueuePorts,
    /// The live queue. The stored copy is written after every change so a crash
    /// costs at most the change that was in flight.
    queue: RwLock<Queue>,
}

impl QueueService {
    /// Wires the service and loads the profile's saved queue if there is one.
    ///
    /// Restoring here rather than in a `restore()` the caller must remember to
    /// call: a queue that only comes back when somebody asks for it is a queue
    /// that comes back for whoever asks first and nobody else.
    pub fn new(
        context: Arc<AppContext>,
        playback: Arc<PlaybackService>,
        ports: QueuePorts,
    ) -> Self {
        // Without a profile there is nothing to restore and nothing that may be
        // saved; the placeholder is replaced the moment one starts playing.
        let profile_id = context.active_profile();
        let restored = profile_id.and_then(|id| ports.queue.load(id).ok().flatten());
        let queue = restored.unwrap_or_else(|| Queue::new(profile_id.unwrap_or_default()));

        Self {
            context,
            playback,
            ports,
            queue: RwLock::new(queue),
        }
    }

    /// Starts a track from the library, with the rest of the library behind it.
    ///
    /// Choosing a row is choosing a starting point, not a single track: the
    /// continuation is what makes "next" mean anything at all.
    pub fn play_from_library(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        if !library
            .iter()
            .any(|summary| summary.media_file_id == media_file_id)
        {
            return Err(CoreError::not_found("track", media_file_id));
        }

        let shuffle = self.with_queue(|queue| queue.shuffle);
        let continuation = self.continuation(&library, media_file_id, shuffle);

        self.write_queue(|queue| {
            queue.profile_id = profile_id;
            queue.start(
                QueueEntry {
                    media_file_id,
                    origin: QueueOrigin::Library,
                },
                continuation,
            );
        });

        self.play_current()
    }

    /// Puts a track at the end of the manual queue.
    pub fn enqueue(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        self.ports
            .tracks
            .summary(profile_id, media_file_id)?
            .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

        self.write_queue(|queue| {
            queue.enqueue(QueueEntry {
                media_file_id,
                origin: QueueOrigin::Library,
            });
        });
        Ok(())
    }

    /// Moves to the next track, or stops when the queue runs out.
    pub fn next(&self) -> Result<()> {
        match self.write_queue(Queue::advance) {
            Some(_) => self.play_current(),
            None => self.playback.stop(),
        }
    }

    /// Restarts the current track, or goes back to the previous one.
    ///
    /// Which of the two is [`previous_action`]'s decision, not this service's:
    /// PROJECT_MASTER 2.3 states it as a rule about elapsed time, and a rule
    /// belongs in a policy.
    pub fn previous(&self) -> Result<()> {
        let position = self.playback.view().position;

        if previous_action(position) == PreviousAction::RestartCurrent {
            return self.playback.seek(PlaybackPosition::START);
        }

        match self.write_queue(Queue::go_back) {
            Some(_) => self.play_current(),
            // Nothing played before this: the start of the track is as far back
            // as "previous" can go.
            None => self.playback.seek(PlaybackPosition::START),
        }
    }

    /// Steps Off -> All -> One -> Off.
    pub fn cycle_repeat(&self) -> Result<()> {
        self.write_queue(|queue| queue.repeat = queue.repeat.next());
        self.persist();
        self.announce();
        Ok(())
    }

    /// Turns shuffle on or off, rebuilding what is still to play.
    ///
    /// Turning it on reorders the continuation that is already queued rather
    /// than the whole library: the listener asked for these tracks in some
    /// order, and shuffle is a statement about the order, not the selection.
    /// Turning it off restores library order from wherever playback has got to.
    pub fn toggle_shuffle(&self) -> Result<()> {
        let shuffle = !self.with_queue(|queue| queue.shuffle);
        let current = self.with_queue(|queue| queue.current);

        let ordered = if shuffle {
            None
        } else {
            // Library order is a question only the library can answer.
            let profile_id = self.context.require_active_profile()?;
            Some(self.ports.tracks.summaries_for_profile(profile_id)?)
        };

        self.write_queue(|queue| {
            queue.shuffle = shuffle;

            match (&ordered, current) {
                (None, _) => {
                    let mut pool: Vec<QueueEntry> = queue.upcoming.iter().copied().collect();
                    shuffle_policy::shuffle(&mut pool, seed());
                    queue.upcoming = pool.into();
                }
                (Some(library), Some(entry)) => {
                    queue.upcoming = continuation_of(library, entry.media_file_id).into();
                }
                (Some(_), None) => {}
            }
        });

        self.persist();
        self.announce();
        Ok(())
    }

    /// Advances when the current track has run out.
    ///
    /// Called from the interface's tick. The engine has no way to call back into
    /// the application layer — the realtime contract of PROJECT_MASTER 8.2
    /// forbids it — so somebody has to ask, and asking four times a second costs
    /// two atomic loads.
    /// Returns whether anything changed, so a caller can redraw only then.
    pub fn poll(&self) -> Result<bool> {
        let view = self.playback.view();

        // Stopped with a track still loaded is the one state that only
        // end-of-track produces: pausing reports paused, and stopping unloads.
        if view.state.has_track() || view.track.is_none() {
            return Ok(false);
        }

        self.next()?;
        Ok(true)
    }

    /// What the transport buttons need to draw themselves.
    ///
    /// Reads no rows: it runs on every tick, and a listing query four times a
    /// second is a database load with nothing to show for it.
    pub fn view(&self) -> QueueView {
        self.with_queue(|queue| QueueView {
            repeat: queue.repeat,
            shuffle: queue.shuffle,
            pending: queue.pending_len(),
            has_previous: !queue.history.is_empty(),
            // Repeat always has somewhere to go: the last track of a repeating
            // list is followed by the first, and a greyed-out next button on it
            // would say otherwise.
            has_next: queue.peek_next().is_some()
                || (queue.repeat != RepeatMode::Off && queue.current.is_some()),
        })
    }

    /// The tracks waiting to play, in the order they will.
    ///
    /// Separate from [`Self::view`] and only called when the queue is on screen.
    pub fn upcoming(&self) -> Result<Vec<TrackSummary>> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        let waiting: Vec<MediaFileId> = self.with_queue(|queue| {
            queue
                .manual
                .iter()
                .chain(queue.upcoming.iter())
                .map(|entry| entry.media_file_id)
                .collect()
        });

        // A queue entry whose file has left the library is skipped rather than
        // reported: it is already unplayable, and a row with no title to draw
        // would be a gap the listener cannot act on.
        Ok(waiting
            .into_iter()
            .filter_map(|id| {
                library
                    .iter()
                    .find(|summary| summary.media_file_id == id)
                    .cloned()
            })
            .collect())
    }

    /// Empties the queue and stops.
    pub fn clear(&self) -> Result<()> {
        let profile_id = self.write_queue(|queue| {
            let profile_id = queue.profile_id;
            *queue = Queue::new(profile_id);
            profile_id
        });
        self.ports.queue.clear(profile_id)?;
        self.playback.stop()
    }

    /// Starts whatever the queue now points at.
    fn play_current(&self) -> Result<()> {
        let Some(entry) = self.with_queue(|queue| queue.current) else {
            return self.playback.stop();
        };

        let played = self.playback.play_track(entry.media_file_id);

        // A file that will not play must not stall the queue: the listener
        // pressed next, and stopping on a missing file would mean pressing it
        // again for every gap in the library.
        if played.is_err() {
            self.persist();
            self.announce();
            return played;
        }

        self.persist();
        self.announce();
        Ok(())
    }

    /// The rest of the library after the chosen track.
    fn continuation(
        &self,
        library: &[TrackSummary],
        from: MediaFileId,
        shuffle: bool,
    ) -> Vec<QueueEntry> {
        let mut entries = continuation_of(library, from);
        if shuffle {
            shuffle_policy::shuffle(&mut entries, seed());
        }
        entries
    }

    fn persist(&self) {
        let queue = self.queue.read().unwrap_or_else(|err| err.into_inner());

        // The placeholder queue of a session with no profile belongs to nobody
        // and has nowhere to go: `queue_state` is keyed by a profile that does
        // not exist.
        if self.context.active_profile() != Some(queue.profile_id) {
            return;
        }

        // A queue that fails to save is not a reason to stop the music. It costs
        // the restore after the next restart, and the listener finds out then
        // rather than mid-track.
        let _ = self.ports.queue.save(&queue);
    }

    fn announce(&self) {
        self.context.events.publish(DomainEvent::QueueChanged);
    }

    fn with_queue<T>(&self, read: impl FnOnce(&Queue) -> T) -> T {
        read(&self.queue.read().unwrap_or_else(|err| err.into_inner()))
    }

    fn write_queue<T>(&self, change: impl FnOnce(&mut Queue) -> T) -> T {
        change(&mut self.queue.write().unwrap_or_else(|err| err.into_inner()))
    }
}

/// The rest of the library after `from`, wrapping round to what precedes it.
///
/// Wrapping rather than stopping at the bottom of the list: starting halfway
/// down a library and never hearing its first half is not what "play from here"
/// means. Every track still appears exactly once, so a round ends where it
/// began and repeat all has a whole library to begin again with.
fn continuation_of(library: &[TrackSummary], from: MediaFileId) -> Vec<QueueEntry> {
    let Some(start) = library
        .iter()
        .position(|summary| summary.media_file_id == from)
    else {
        return Vec::new();
    };

    library[start + 1..]
        .iter()
        .chain(&library[..start])
        .map(|summary| QueueEntry {
            media_file_id: summary.media_file_id,
            origin: QueueOrigin::Library,
        })
        .collect()
}

/// A seed for the shuffle.
///
/// From a fresh identifier because that is the only entropy the core layer
/// has: a clock would give a seed an attacker could guess, which does not
/// matter here, but would also give two shuffles in the same millisecond the
/// same order, which does.
fn seed() -> u64 {
    (Uuid::new_v4().as_u128() >> 64) as u64
}
