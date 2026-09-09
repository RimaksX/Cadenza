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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::application::context::AppContext;
use crate::application::view_state::QueueView;
use crate::domain::ids::{MediaFileId, PlaylistId, RadioSessionId};
use crate::domain::media_file::FileState;
use crate::domain::policies::playback_policy::{
    PreviousAction, next_in_library, previous_action, transition_for,
};
use crate::domain::policies::shuffle_policy;
use crate::domain::policies::shuffle_policy::{ARTIST_COOLDOWN, Candidate};
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::ports::repositories::{
    PlaylistRepositoryPort, QueueRepositoryPort, TrackFeaturesRepositoryPort, TrackRepositoryPort,
};
use crate::domain::queue::{Queue, QueueEntry, QueueOrigin, RepeatMode};
use crate::domain::radio::{MIN_BATCH_SIZE, REFILL_THRESHOLD};
use crate::domain::stats::PlaySource;
use crate::domain::track::{TrackFeatures, TrackSummary};
use crate::{CoreError, Result};

use super::PlaybackService;

/// Everything the queue talks to.
pub struct QueuePorts {
    /// Where the queue is kept between runs.
    pub queue: Arc<dyn QueueRepositoryPort>,
    /// The library, which is the pool the continuation is built from.
    pub tracks: Arc<dyn TrackRepositoryPort>,
    /// The lists, for the one a listener is playing through.
    ///
    /// The repository rather than the service: what the queue needs of a
    /// playlist is the order of its entries, and the service around it is about
    /// covers, names and folders (`MASTER_ISSUES` 136).
    pub playlists: Arc<dyn PlaylistRepositoryPort>,
    /// What the library sounds like, for shuffle to choose by (PROJECT_MASTER
    /// 9.3). A library nobody has analysed yet still shuffles: every candidate
    /// simply scores the same.
    pub features: Arc<dyn TrackFeaturesRepositoryPort>,
    /// The station, when there is one to keep topped up.
    ///
    /// Optional because the queue is older than radio and works without it: a
    /// command line that lists tracks has no station, and neither has a test
    /// about repeat modes.
    pub radio: Option<Arc<super::RadioService>>,
    /// The equaliser, to be told which track it is now playing.
    ///
    /// The queue is the only thing that knows a track has started — the engine
    /// knows it too, but seconds earlier and on a thread that must not touch a
    /// database. Optional for the same reason radio is: a test about repeat
    /// modes has no filters to set.
    pub eq: Option<Arc<super::EqService>>,
}

/// The playback queue and the transport commands that move through it.
pub struct QueueService {
    context: Arc<AppContext>,
    playback: Arc<PlaybackService>,
    ports: QueuePorts,
    /// The live queue. The stored copy is written after every change so a crash
    /// costs at most the change that was in flight.
    queue: RwLock<Queue>,
    /// How many joins the engine had made the last time anybody looked.
    seen_advances: AtomicU64,
    /// What the library plays after the track in it, worked out once.
    ///
    /// Kept for two reasons. It is asked for on every tick while nothing is
    /// armed, and listing a library four times a second is a database kept busy
    /// saying the same thing. And under shuffle the answer is a throw of the
    /// dice: asking twice would arm one track and move the queue to another.
    next_up: RwLock<Option<(QueueEntry, Option<QueueEntry>)>>,
    /// The entry the engine was last told to open.
    ///
    /// The engine knows *that* something is armed and this knows *what*, which
    /// is what lets a queue changed mid-track replace it: a track queued by
    /// hand thirty seconds before the end has to be the one that plays next,
    /// not the one that was decoded before it was asked for.
    armed: RwLock<Option<QueueEntry>>,
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
        let mut queue = restored.unwrap_or_else(|| Queue::new(profile_id.unwrap_or_default()));

        // The continuation lane belongs to a station and to nothing else. The
        // library stopped being written into it when what follows a library
        // track began to be worked out on demand; a playlist stopped when it
        // became a source that reads itself, and a queue saved before that
        // still holds the whole list there (`MASTER_ISSUES` 136). Both go.
        // Nothing is lost with them: the source plays on from the track that is
        // playing. The manual queue, which is the part somebody actually wrote,
        // comes back untouched.
        queue
            .upcoming
            .retain(|entry| matches!(entry.origin, QueueOrigin::Radio(_)));

        Self {
            context,
            playback,
            ports,
            queue: RwLock::new(queue),
            seen_advances: AtomicU64::new(0),
            next_up: RwLock::new(None),
            armed: RwLock::new(None),
        }
    }

    /// Loads the active profile's queue, dropping whoever else's was held.
    ///
    /// The third of PROJECT_MASTER 2.5's switching steps, for the one piece of
    /// state that does not carry its owner with it. The equaliser and the
    /// playback settings cache the profile alongside the value and notice a
    /// switch by themselves; a queue is a queue, and the only thing that says
    /// whose it is, is which profile was active when it was read.
    ///
    /// Nothing is saved here: the outgoing queue was written after the change
    /// that last touched it, which is what `persist` is for.
    pub fn reload(&self) {
        let profile_id = self.context.active_profile();
        let restored = profile_id.and_then(|id| self.ports.queue.load(id).ok().flatten());
        let mut queue = restored.unwrap_or_else(|| Queue::new(profile_id.unwrap_or_default()));
        queue
            .upcoming
            .retain(|entry| matches!(entry.origin, QueueOrigin::Radio(_)));

        *self.queue.write().unwrap_or_else(|err| err.into_inner()) = queue;

        // Everything remembered about the queue that just left belonged to it.
        *self.next_up.write().unwrap_or_else(|err| err.into_inner()) = None;
        *self.armed.write().unwrap_or_else(|err| err.into_inner()) = None;
        self.seen_advances
            .store(self.playback.advances(), Ordering::Relaxed);

        self.announce();
    }

    /// Starts a track from the library, and lets the library carry on behind it.
    ///
    /// Nothing is queued by this: the queue holds what the listener chose to
    /// hear next, and the rest of the library is not that — it is simply what
    /// comes after, which [`next_in_library`] can work out whenever it is asked
    /// (MASTER_ISSUES 45). Anything already queued by hand still plays first.
    pub fn play_from_library(&self, media_file_id: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        if !library
            .iter()
            .any(|summary| summary.media_file_id == media_file_id)
        {
            return Err(CoreError::not_found("track", media_file_id));
        }

        self.end_station();

        self.write_queue(|queue| {
            queue.profile_id = profile_id;
            // An empty continuation also ends a playlist that was playing: the
            // listener pointed somewhere else, and the rest of that list is no
            // longer what follows.
            queue.start(
                QueueEntry {
                    media_file_id,
                    origin: QueueOrigin::Library,
                },
                Vec::new(),
            );
        });

        self.play_current()
    }

    /// Starts a playlist at one of its tracks.
    ///
    /// The list is named, not handed over: what it holds is read from the list
    /// itself, which is the only copy that cannot be stale. A caller passing
    /// its own idea of the tracks was a second answer to a question with one
    /// (`MASTER_ISSUES` 136).
    ///
    /// The entry carries the playlist as its origin, which is what makes the
    /// transition between its tracks gapless rather than crossfaded
    /// (PROJECT_MASTER 2.4), what tells the continuation which rows to read,
    /// and what a restored queue needs to still know it is playing a playlist
    /// rather than a library.
    pub fn play_playlist(&self, playlist_id: PlaylistId, from: MediaFileId) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;

        let items = self.ports.playlists.items(playlist_id)?;
        if !items.iter().any(|item| item.media_file_id == from) {
            return Err(CoreError::not_found("track", from));
        }

        self.end_station();

        // Nothing is dealt into the queue. The playlist plays itself, the same
        // way the library does: what follows this track is the next entry of
        // the list, read when it is wanted. Dealing the rest into `upcoming`
        // was what made starting a playlist look like it had filled the
        // listener's own queue with forty tracks nobody put there
        // (`MASTER_ISSUES` 136).
        let origin = QueueOrigin::Playlist(playlist_id);

        self.write_queue(|queue| {
            queue.profile_id = profile_id;
            queue.start(
                QueueEntry {
                    media_file_id: from,
                    origin,
                },
                Vec::new(),
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

        self.persist();
        self.announce();
        Ok(())
    }

    /// Jumps to an entry by where it appears in [`Self::upcoming`], keeping the
    /// rest of the queue.
    ///
    /// Everything skipped over becomes history rather than staying in front:
    /// the listener passed those tracks, and "previous" is what walks back
    /// through them.
    pub fn play_at(&self, position: usize) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        let Some(index) = self.with_queue(|queue| locate(queue, position, &library)) else {
            return Err(CoreError::not_found("queue entry", position));
        };

        self.write_queue(|queue| {
            // Everything stepped over has had its turn in this round as well
            // as gone into the back-stack: shuffle must not offer a track the
            // listener has just skipped past.
            if let Some(leaving) = queue.current.take() {
                queue.history.push(leaving);
                queue.round.push(leaving);
            }

            let mut passed: Vec<QueueEntry> = queue.manual.drain(..=index).collect();
            let target = passed.pop().expect("the range ends at the target");
            queue.round.extend(passed.iter().copied());
            queue.history.extend(passed);
            queue.current = Some(target);
        });

        self.play_current()
    }

    /// Takes one entry out of the queue by where it appears in [`Self::upcoming`].
    ///
    /// By position rather than by track, because the same track may legitimately
    /// be waiting twice and the listener pointed at one of them. The position is
    /// counted over the same filtered list the interface drew, so the row that
    /// disappears is the row that was clicked.
    pub fn remove_at(&self, position: usize) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        let Some(index) = self.with_queue(|queue| locate(queue, position, &library)) else {
            return Err(CoreError::not_found("queue entry", position));
        };

        self.write_queue(|queue| queue.manual.remove(index));

        self.persist();
        self.announce();
        Ok(())
    }

    /// Moves to the next track, or stops when there is nothing left to play.
    pub fn next(&self) -> Result<()> {
        // For the same reason as `previous`: skipping in the last seconds of a
        // track would otherwise step past a join the engine had already made,
        // and skip two.
        self.catch_up()?;

        match self.step()? {
            Some(_) => self.play_current(),
            None => self.playback.stop(),
        }
    }

    /// Moves on by one: to what was queued, or to what the library holds next.
    ///
    /// The queue is asked first because it is the listener's own list, and the
    /// library is only what happens in the absence of one.
    fn step(&self) -> Result<Option<QueueEntry>> {
        if self.with_queue(|queue| queue.following().is_some()) {
            return Ok(self.write_queue(|queue| queue.advance(seed())));
        }

        match self.library_successor()? {
            Some(next) => {
                self.write_queue(|queue| queue.move_to(next));
                Ok(Some(next))
            }
            // Nothing queued and nothing after it: `advance` is what puts the
            // finished track into history on the way to stopping.
            None => Ok(self.write_queue(|queue| queue.advance(seed()))),
        }
    }

    /// The track that follows the one playing, without moving to it.
    fn successor(&self) -> Result<Option<QueueEntry>> {
        match self.with_queue(Queue::following) {
            Some(queued) => Ok(Some(queued)),
            None => self.library_successor(),
        }
    }

    /// What the source now playing offers after the current track.
    ///
    /// The library and a playlist both continue by themselves, and by one rule:
    /// the row after this one, or a row nobody has heard this round where
    /// shuffle is on. A station does not come through here — it keeps its own
    /// picks in the queue, because generating one costs a pass over the library
    /// and that is not work for the gap between two tracks.
    fn library_successor(&self) -> Result<Option<QueueEntry>> {
        let Some(current) = self.with_queue(|queue| queue.current) else {
            return Ok(None);
        };

        let list = match current.origin {
            QueueOrigin::Library => None,
            QueueOrigin::Playlist(playlist_id) => Some(playlist_id),
            QueueOrigin::Radio(_) => return Ok(None),
        };

        if let Some((track, next)) = self.remembered_successor()
            && track == current
        {
            return Ok(next);
        }

        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        // The rows this source is made of. For a playlist, its own entries in
        // its own order — resolved through the library listing that was read
        // anyway, so a list is one query rather than two. An entry whose file
        // has left the library is skipped here, exactly as the queue screen
        // skips it: it is already unplayable.
        let library = match list {
            None => library,
            Some(playlist_id) => {
                let mut items = self.ports.playlists.items(playlist_id)?;
                items.sort_by_key(|item| item.position);

                // The first appearance of each file and no other. A playlist
                // may legitimately hold the same track twice, and a queue entry
                // records only which file is playing — so a second copy is a
                // row nothing can tell apart from the first, and "the row after
                // this one" would answer with the first one's successor for
                // ever. In `A B A C` that is `A B A B A B`, with `C` never
                // reached. Each track once, in the order it first appears.
                let mut seen = HashSet::new();
                items
                    .iter()
                    .filter(|item| seen.insert(item.media_file_id))
                    .filter_map(|item| {
                        library
                            .iter()
                            .find(|summary| summary.media_file_id == item.media_file_id)
                            .cloned()
                    })
                    .collect()
            }
        };

        // The round, not the back-stack. What shuffle must not offer again is
        // what this pass has already played; where the listener has been over
        // every session is a different question, and answering it here is what
        // left a library of forty-one with a hundred and thirty-seven tracks
        // "already heard" and shuffle with nothing to choose
        // (`MASTER_ISSUES` 135).
        let (played, shuffle, repeat) = self.with_queue(|queue| {
            let played: Vec<MediaFileId> = queue
                .round
                .iter()
                .map(|entry| entry.media_file_id)
                .collect();
            (played, queue.shuffle, queue.repeat)
        });

        let chosen = if shuffle {
            self.shuffled_successor(&library, current.media_file_id, &played, repeat)?
        } else {
            let order: Vec<MediaFileId> = library
                .iter()
                .map(|summary| summary.media_file_id)
                .collect();
            next_in_library(&order, current.media_file_id, repeat)
        };

        let next = chosen.map(|media_file_id| QueueEntry {
            media_file_id,
            origin: current.origin,
        });

        *self.next_up.write().unwrap_or_else(|err| err.into_inner()) = Some((current, next));
        Ok(next)
    }

    /// What shuffle picks out of the library.
    ///
    /// The round is what has not been heard yet; when it empties, repeat all
    /// begins another and repeat off stops — the fourth hard rule of
    /// PROJECT_MASTER 9.2. Which of the round's tracks plays is
    /// [`shuffle_policy::choose_next`]'s decision and not this service's: it
    /// scores every candidate against what is playing, keeps the best handful
    /// and draws from those, which is 9.4.
    fn shuffled_successor(
        &self,
        library: &[TrackSummary],
        current: MediaFileId,
        played: &[MediaFileId],
        repeat: RepeatMode,
    ) -> Result<Option<MediaFileId>> {
        // Sets rather than scans: this runs over the whole library, and a
        // session's history is as long as the session. Two linear passes
        // instead of a product of two lists that both grow.
        let heard: HashSet<MediaFileId> = played.iter().copied().collect();

        let unheard: Vec<&TrackSummary> = library
            .iter()
            .filter(|summary| {
                summary.media_file_id != current && !heard.contains(&summary.media_file_id)
            })
            .collect();

        let round: Vec<&TrackSummary> = if unheard.is_empty() {
            if repeat != RepeatMode::All {
                return Ok(None);
            }
            // Everything has had a turn. A new round is the whole library —
            // except what is playing, unless that is all there is.
            let again: Vec<&TrackSummary> = library
                .iter()
                .filter(|summary| summary.media_file_id != current)
                .collect();
            if again.is_empty() {
                library.iter().collect()
            } else {
                again
            }
        } else {
            unheard
        };

        // Read once for the whole round rather than per candidate. Only files
        // that have been analysed appear here; the rest score neutrally and
        // still get their turn.
        let features = self.ports.features.list_all()?;
        let analysed: HashMap<MediaFileId, &TrackFeatures> = features
            .iter()
            .map(|row| (row.media_file_id, row))
            .collect();
        let find = |id: MediaFileId| analysed.get(&id).copied();

        let candidates: Vec<Candidate<'_>> = round
            .iter()
            .map(|summary| Candidate {
                media_file_id: summary.media_file_id,
                // A listing already excludes what has left the library. What it
                // cannot see is a file that has gone missing since, and that is
                // the scanner's business rather than the queue's.
                file_state: FileState::Available,
                artist: summary.artist.as_deref(),
                features: find(summary.media_file_id),
            })
            .collect();

        // The last few artists, most recent last, which is the order the
        // cooldown reads them in.
        let recent: Vec<Option<&str>> = played
            .iter()
            .rev()
            .take(ARTIST_COOLDOWN)
            .rev()
            .map(|id| {
                library
                    .iter()
                    .find(|summary| summary.media_file_id == *id)
                    .and_then(|summary| summary.artist.as_deref())
            })
            .collect();

        Ok(shuffle_policy::choose_next(
            find(current),
            &candidates,
            &recent,
            seed(),
        ))
    }

    fn remembered_successor(&self) -> Option<(QueueEntry, Option<QueueEntry>)> {
        *self.next_up.read().unwrap_or_else(|err| err.into_inner())
    }

    /// Restarts the current track, or goes back to the previous one.
    ///
    /// Which of the two is [`previous_action`]'s decision, not this service's:
    /// PROJECT_MASTER 2.3 states it as a rule about elapsed time, and a rule
    /// belongs in a policy.
    pub fn previous(&self) -> Result<()> {
        // Reconciled first, for the window after the callback has crossed a
        // join but before the tick has noticed: a quarter of a second in which
        // stepping again would skip a track nobody asked to skip.
        self.catch_up()?;

        let position = self.playback.view().position;

        if previous_action(position) == PreviousAction::RestartCurrent {
            // Loaded, not seeked, and that is the whole of this defect.
            //
            // The decoder runs ahead of the speakers by design, so in the last
            // seconds of a track it has often already joined to the next one.
            // A seek acts on what the *decoder* holds — which by then is that
            // next track, and seeking it to the start is how "previous" came
            // to begin the following song. Loading acts on what the *queue*
            // holds, and the queue still holds the track being heard.
            //
            // It costs opening the file again where a seek would have done.
            // That is what starting any track costs, and it buys the command
            // meaning the same thing two seconds before the end as two minutes
            // before it.
            return self.play_current();
        }

        match self.write_queue(Queue::go_back) {
            Some(_) => self.play_current(),
            // Nothing played before this: the start of the track is as far back
            // as "previous" can go.
            None => self.play_current(),
        }
    }

    /// Steps Off -> All -> One -> Off.
    pub fn cycle_repeat(&self) -> Result<()> {
        self.write_queue(|queue| queue.repeat = queue.repeat.next());
        self.persist();
        self.announce();
        Ok(())
    }

    /// Turns shuffle on or off.
    ///
    /// What it governs is what plays *after* what is queued: the source behind
    /// the queue — the library, or the playlist being played through — comes at
    /// random rather than in order. Nothing in the queue is reordered. The
    /// manual queue was put in an order by hand and shuffle is not an
    /// instruction to undo that; a station's picks were already chosen for the
    /// listener, and shuffling a choice is not a second choice.
    ///
    /// So this writes one flag and nothing else. `announce` forgets the
    /// successor that was worked out under the old setting, and the next tick
    /// arms whatever the new one picks — which is why pressing it takes effect
    /// on the track after this one rather than at the end of a list.
    pub fn toggle_shuffle(&self) -> Result<()> {
        let shuffle = !self.with_queue(|queue| queue.shuffle);
        self.write_queue(|queue| queue.shuffle = shuffle);

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
        let mut changed = self.catch_up()?;

        // Before anything decides there is nowhere to go: a station that has
        // run low tops itself up, which is what makes radio endless (10.5).
        changed |= self.refill_radio()?;

        let view = self.playback.view();
        // Stopped with a track still loaded is the one state that only
        // end-of-track produces: pausing reports paused, and stopping unloads.
        // It now means the track ran out with nothing armed behind it — the end
        // of the queue, or a file that would not open when it was armed.
        if !view.state.has_track() && view.track.is_some() {
            self.next()?;
            changed = true;
        }

        // Arm whatever follows. Doing it here rather than at every change to
        // the queue costs one atomic read per tick and cannot be forgotten: the
        // engine drops what it had armed whenever the ground moves under it,
        // and this asks it rather than trying to remember for it.
        self.arm_next()?;

        Ok(changed)
    }

    /// Starts a station: its first batch becomes the continuation.
    ///
    /// The manual queue survives, because it outranks radio (PROJECT_MASTER
    /// 10.5) — a track queued by hand plays before whatever the station chose.
    pub fn play_radio(&self, session_id: RadioSessionId, batch: &[MediaFileId]) -> Result<()> {
        let profile_id = self.context.require_active_profile()?;
        let Some((first, rest)) = batch.split_first() else {
            return Err(CoreError::invalid(
                "radio",
                "the station found nothing to play",
            ));
        };

        let origin = QueueOrigin::Radio(session_id);
        self.write_queue(|queue| {
            queue.profile_id = profile_id;
            queue.start(
                QueueEntry {
                    media_file_id: *first,
                    origin,
                },
                rest.iter()
                    .map(|media_file_id| QueueEntry {
                        media_file_id: *media_file_id,
                        origin,
                    })
                    .collect(),
            );
        });

        self.play_current()
    }

    /// Ends the station, because something else is playing now.
    ///
    /// Choosing a track or a playlist is how a listener says they are done with
    /// radio; there is no separate way to say it and there does not need to be.
    /// What the station already queued is left alone — it is still music, and
    /// the listener will pass it on their way out.
    fn end_station(&self) {
        if let Some(radio) = self.ports.radio.as_ref() {
            radio.stop();
        }
    }

    /// Tops the station up before it runs dry.
    ///
    /// Asked on every tick and answered by two reads in the ordinary case. The
    /// threshold is what keeps generation off the transition: refilling at the
    /// last track would put a database query and a scoring pass exactly where
    /// the next track is supposed to start.
    fn refill_radio(&self) -> Result<bool> {
        let Some(radio) = self.ports.radio.as_ref() else {
            return Ok(false);
        };

        // Asked of the service and not only of the queue: a station that has
        // ended still has its picks in the lane, and asking it for more would
        // be a question with no answer — four times a second, for as long as
        // they play.
        if radio.session().is_none() {
            return Ok(false);
        }

        let Some(session_id) = self.with_queue(|queue| match queue.current {
            Some(QueueEntry {
                origin: QueueOrigin::Radio(session_id),
                ..
            }) => Some(session_id),
            _ => None,
        }) else {
            return Ok(false);
        };

        // Only the station's own lane counts. A listener who queued ten tracks
        // by hand has not thereby told the station it may stop generating.
        let waiting = self.with_queue(|queue| {
            queue
                .upcoming
                .iter()
                .filter(|entry| matches!(entry.origin, QueueOrigin::Radio(_)))
                .count()
        });
        if waiting > REFILL_THRESHOLD {
            return Ok(false);
        }

        let batch = radio.next_batch(MIN_BATCH_SIZE)?;
        if batch.is_empty() {
            // The library has nothing left the station has not offered. It ends
            // when the lane empties, which is what the queue already does.
            return Ok(false);
        }

        self.write_queue(|queue| {
            for media_file_id in &batch {
                queue.upcoming.push_back(QueueEntry {
                    media_file_id: *media_file_id,
                    origin: QueueOrigin::Radio(session_id),
                });
            }
        });

        self.persist();
        self.announce();
        Ok(true)
    }

    /// Brings the queue's own bookkeeping up to what the engine already played.
    ///
    /// A join makes no silence and asks no permission, so nothing here may
    /// touch the engine: the audio has moved on, and loading the track that is
    /// already playing would flush the ring and put a hole in the middle of the
    /// handover that was the whole point.
    fn catch_up(&self) -> Result<bool> {
        let advances = self.playback.advances();
        let seen = self.seen_advances.swap(advances, Ordering::Relaxed);
        if seen >= advances {
            return Ok(false);
        }

        // The other half of the probe in `PlaybackService::seek`
        // (MASTER_ISSUES 83). A step the listener did not ask for, logged with
        // the counts that caused it: a seek immediately followed by one of
        // these is the defect, and nothing short of a real machine produces the
        // pair.
        self.context.info(&format!(
            "the engine advanced on its own: {} counted, {seen} seen",
            advances
        ));

        for _ in seen..advances {
            self.step()?;
        }

        if let Some(entry) = self.with_queue(|queue| queue.current) {
            self.playback
                .adopt(entry.media_file_id, source_of(entry.origin))?;
            // Late by however long the ring buffer is, because that is when
            // anybody above the engine finds out a gapless join happened at
            // all. The alternative is setting filters from the audio thread.
            self.follow_with_eq(entry.media_file_id);
        }
        self.persist();
        self.announce();
        Ok(true)
    }

    /// Opens the track that follows the one playing, so the join can be decoded
    /// before it is needed.
    ///
    /// The transition is chosen from what is *playing*, not from what is
    /// coming: PROJECT_MASTER 2.4 is a rule about the material being listened
    /// to, and a playlist does not start fading out because the next thing was
    /// queued by hand.
    fn arm_next(&self) -> Result<()> {
        // With nothing playing there is nothing to arm, and a window opened
        // before anybody has chosen a profile has no settings to read either.
        let Some(current) = self.with_queue(|queue| queue.current) else {
            return Ok(());
        };

        // Asked before the successor is: working out what follows may cost a
        // listing, and a listener who has turned preloading off is not going to
        // be given one four times a second for nothing.
        let settings = self.playback.settings()?;
        if !settings.preload_next {
            return Ok(());
        }

        let Some(following) = self.successor()? else {
            return Ok(());
        };

        // Already open, and still the right track. Asking every tick costs one
        // atomic read and one lock, and cannot be forgotten: the engine drops
        // what it had armed whenever the ground moves under it — a seek, a new
        // track loaded — and only the engine knows when that was.
        if self.playback.armed() && self.armed_entry() == Some(following) {
            return Ok(());
        }

        // A file that will not open is not a reason to interrupt what is
        // playing. It stays unarmed, the track ends the ordinary way, and the
        // failure is reported then — by the load that also fails.
        if self
            .playback
            .preload(
                following.media_file_id,
                transition_for(current.origin, &settings),
            )
            .is_ok()
        {
            *self.armed.write().unwrap_or_else(|err| err.into_inner()) = Some(following);
        }
        Ok(())
    }

    fn armed_entry(&self) -> Option<QueueEntry> {
        *self.armed.read().unwrap_or_else(|err| err.into_inner())
    }

    /// What the transport buttons need to draw themselves.
    ///
    /// Reads no rows: it runs on every tick, and a listing query four times a
    /// second is a database load with nothing to show for it.
    pub fn view(&self) -> QueueView {
        self.with_queue(|queue| QueueView {
            repeat: queue.repeat,
            shuffle: queue.shuffle,
            // The manual queue alone, because that is what the queue screen
            // lists and what a count beside it should count.
            pending: queue.manual.len(),
            has_previous: !queue.history.is_empty(),
            // Anything playing has somewhere to go: the library follows it, and
            // repeat sends the last track of a list back to the first. The one
            // case this overstates — the bottom of the library with repeat off
            // — costs a press that stops the music, which is what the button
            // would have done there anyway. Finding out for certain means
            // listing the library, and this runs on every tick.
            has_next: queue.peek_next().is_some() || queue.current.is_some(),
        })
    }

    /// The tracks waiting to play, in the order they will.
    ///
    /// Separate from [`Self::view`] and only called when the queue is on screen.
    pub fn upcoming(&self) -> Result<Vec<TrackSummary>> {
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        // What the listener put here, and nothing else. A station keeps a few
        // picks ahead of the needle in the other lane; those are the station
        // playing, not a list anybody wrote, and showing them made the queue
        // look like something that fills itself (`MASTER_ISSUES` 136).
        let waiting: Vec<MediaFileId> = self.with_queue(|queue| {
            queue
                .manual
                .iter()
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

    /// Empties what is waiting, and leaves what is playing alone.
    ///
    /// Clearing is a statement about the list, not about the music. The track
    /// that is playing is not in the list — it left the queue when it started —
    /// so there is nothing here to stop, and the library carries on behind it
    /// exactly as it would have if the queue had always been empty.
    pub fn clear(&self) -> Result<()> {
        self.write_queue(|queue| {
            queue.manual.clear();
            queue.upcoming.clear();
        });

        self.persist();
        self.announce();
        Ok(())
    }

    /// Starts whatever the queue now points at.
    fn play_current(&self) -> Result<()> {
        let Some(entry) = self.with_queue(|queue| queue.current) else {
            return self.playback.stop();
        };

        // Before the first sample rather than after it: the sound a listener
        // chose for this record is part of how it starts (`MASTER_ISSUES` 89).
        self.follow_with_eq(entry.media_file_id);

        let played = self
            .playback
            .play_track(entry.media_file_id, source_of(entry.origin));

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

    /// Tells the equaliser what is playing now, if there is one to tell.
    ///
    /// A failure here is not a reason to stop the music: the worst it costs is
    /// the previous track's curve on this one, which the listener can see and
    /// change, whereas a track that will not start is a track that will not
    /// start.
    fn follow_with_eq(&self, media_file_id: MediaFileId) {
        let Some(eq) = self.ports.eq.as_ref() else {
            return;
        };

        if let Err(err) = eq.follow(media_file_id) {
            self.context
                .warn(&format!("the equaliser was not set for this track: {err}"));
        }
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
        // rather than mid-track — so the log is where they find out sooner.
        if let Err(err) = self.ports.queue.save(&queue) {
            self.context
                .warn(&format!("the queue was not saved: {err}"));
        }
    }

    fn announce(&self) {
        // Any change to the queue can change what follows, so the remembered
        // answer goes with it. A library that changes under a playing track is
        // not covered: that one is seen by the track after next, which is the
        // price of not listing the library on every tick.
        *self.next_up.write().unwrap_or_else(|err| err.into_inner()) = None;
        self.context.events.publish(DomainEvent::QueueChanged);
    }

    fn with_queue<T>(&self, read: impl FnOnce(&Queue) -> T) -> T {
        read(&self.queue.read().unwrap_or_else(|err| err.into_inner()))
    }

    fn write_queue<T>(&self, change: impl FnOnce(&mut Queue) -> T) -> T {
        change(&mut self.queue.write().unwrap_or_else(|err| err.into_inner()))
    }
}

/// What the history calls the place a track came from.
///
/// [`PlaySource::Manual`] has no origin of its own: a track queued by hand
/// carries the origin of wherever it was queued *from*, which is the library.
/// Telling the two apart would mean remembering which lane an entry came out
/// of, and nothing counts them separately yet.
const fn source_of(origin: QueueOrigin) -> PlaySource {
    match origin {
        QueueOrigin::Library => PlaySource::Library,
        QueueOrigin::Playlist(_) => PlaySource::Playlist,
        QueueOrigin::Radio(session_id) => PlaySource::Radio(session_id),
    }
}

/// Which entry of the manual queue the interface's row at `position` is.
///
/// Only the manual queue, because only the manual queue is drawn: a position
/// comes from a row somebody pressed, and there are no rows for the lane a
/// station keeps its picks in (`MASTER_ISSUES` 136).
///
/// Not simply `position`: the listing skips entries whose file has left the
/// library, so a row's place on screen is not its place in the lane. Both walks
/// — removing and jumping — have to count the way the drawing counted, or they
/// act on the wrong track.
fn locate(queue: &Queue, position: usize, library: &[TrackSummary]) -> Option<usize> {
    let playable = |entry: &QueueEntry| {
        library
            .iter()
            .any(|summary| summary.media_file_id == entry.media_file_id)
    };

    queue
        .manual
        .iter()
        .enumerate()
        .filter(|(_, entry)| playable(entry))
        .map(|(index, _)| index)
        .nth(position)
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
