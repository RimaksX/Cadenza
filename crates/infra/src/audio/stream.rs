//! The playing stream: what the threads share, and what each of them does.
//!
//! Three threads meet here and only one of them is realtime:
//!
//! * the **control** thread calls the engine and sends commands — load, seek,
//!   stop;
//! * the **decode** thread ([`decode_loop`]) opens files, decodes, resamples and
//!   fills the ring;
//! * the **audio callback** ([`fill_output`]) empties the ring into the device.
//!
//! Only the last one is bound by section 8.2, and everything it is allowed to do
//! is in one function: read atomics, read the ring, multiply. No allocation, no
//! locking, no IO, no database, no UI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cadenza_core::domain::eq::EqMode;
use cadenza_core::domain::playback::TransitionProfile;
use cadenza_core::domain::settings::DEFAULT_CROSSFADE;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};

use super::crossfade::{equal_power, fade_length};
use super::eq::{EqChain, MAX_BANDS};
use super::resampler::Resampling;
use super::ring_buffer::SampleRing;
use super::symphonia_decoder::{StreamInfo, TrackStream};

/// How far ahead of the device the decoder is allowed to run.
///
/// Two seconds absorbs a slow disk, a stalled thread and a background scan
/// without becoming a delay the listener can feel: nothing here is heard late,
/// it is only decoded early.
const PREBUFFER_SECONDS: u32 = 2;

/// How much must be decoded before the device is allowed to start consuming.
///
/// Without it the first callback arrives before the decoder has produced
/// anything and every track — and every seek — begins with an underrun. Fifty
/// milliseconds covers the first packet of the slowest format here and is short
/// enough that pressing play still feels immediate.
const PRIME_SECONDS: f32 = 0.05;

/// How long a gain change takes to arrive.
///
/// Five milliseconds is short enough to feel immediate on a volume slider and
/// long enough that the step is inaudible. Jumping straight to the new value
/// would put a discontinuity in the waveform, which is what a click is
/// (PROJECT_MASTER 8.3).
const RAMP_SECONDS: f32 = 0.005;

/// How long the decoder waits for the callback to acknowledge a flush.
///
/// Bounded, because the callback might not be running at all — a device that
/// disappeared, a stream that failed to start — and a seek must not hang the
/// caller forever over it.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(200);

/// How long the decode thread sleeps when there is nothing useful to do.
const IDLE_NAP: Duration = Duration::from_millis(3);

/// Stands for "no track is waiting". A real boundary is a frame index, and
/// output will not reach this one in any listening lifetime.
const NO_BOUNDARY: u64 = u64::MAX;

/// How much of what was played the visualiser can look back at.
///
/// A fifth of a second at any rate anyone plays. The reader takes the newest
/// window it needs and throws the rest away, so this only has to cover the gap
/// between two of its readings without the callback running out of room.
const TAP_FRAMES: usize = 8_192;

/// How many samples the decoder mixes in one pass.
///
/// Small enough that a crossfade's gain is recomputed often, large enough that
/// the ring is not poked a hundred times a second. It is rounded down to whole
/// frames before use.
const BLOCK_SAMPLES: usize = 4_096;

/// State shared by the control thread, the decode thread and the callback.
#[derive(Debug)]
pub(crate) struct Shared {
    /// Decoded samples waiting to be played.
    pub(crate) ring: SampleRing,
    /// Output sample rate. Fixed for the life of the engine.
    pub(crate) rate: u32,
    /// Output channel count. Fixed for the life of the engine.
    pub(crate) channels: u16,
    /// Whether the callback should be consuming.
    pub(crate) playing: AtomicBool,
    /// Whether enough has been decoded for the device to start.
    ///
    /// Cleared on every flush, so a seek refills before it plays instead of
    /// starting on whatever fragment happened to arrive first.
    primed: AtomicBool,
    /// Samples that have to be queued before playback starts.
    prime_samples: usize,
    /// Output amplitude, as the bits of an `f32`. Already tapered.
    volume: AtomicU32,
    /// Frames handed to the device since the last flush base.
    ///
    /// Absolute: it counts output, not the position in any one track, and it is
    /// never rewound by a transition. Where the current track started in this
    /// count is [`Self::track_base`], and the difference is the position.
    frames_played: AtomicU64,
    /// Frames the decoder has pushed, counted the same way.
    ///
    /// The decoder's end of the same ruler. It is what lets it say "the next
    /// track begins at frame N" in a number the callback can compare against.
    frames_pushed: AtomicU64,
    /// Where the track now playing began.
    track_base: AtomicU64,
    /// Where the next track's own clock starts.
    ///
    /// Published by the decoder as it mixes the join. That is what makes the
    /// handover sample-accurate: nobody has to notice it in time, because the
    /// frame it happens on was decided before the samples were queued.
    boundary: AtomicU64,
    /// Where the handover should be *said*, or [`u64::MAX`] while none is
    /// coming.
    ///
    /// Not the same frame. A gapless join is one instant and both are it, but a
    /// crossfade is four seconds long, and for the first half of it the track
    /// still louder is the one leaving. Announcing at the start would name the
    /// incoming track over audio that is mostly the outgoing one; the middle is
    /// where what is heard changes over, so the middle is where the window is
    /// told. The clock is still rebased to [`Self::boundary`], so the position
    /// shown at that moment is the truthful two seconds in.
    announce_at: AtomicU64,
    /// Length of the track waiting at [`Self::boundary`].
    boundary_duration_ms: AtomicU64,
    /// How many times output has crossed a boundary.
    ///
    /// The application watches this number: it is how the queue learns that the
    /// track it thinks is playing has already handed over.
    pub(crate) advances: AtomicU64,
    /// Crossfade length in milliseconds, as last set through the port.
    pub(crate) crossfade_ms: AtomicU64,
    /// Which set of controls the equaliser is using, as an [`EqMode`] index.
    eq_mode: AtomicU8,
    /// Where each band sits, in hertz.
    ///
    /// Fixed arrays rather than a lock: the callback reads these, and a lock on
    /// the realtime path is the one thing section 8.2 has no exception for.
    /// Simple mode uses the first three.
    eq_frequencies: [AtomicU32; MAX_BANDS],
    /// How wide each band is, as the bits of an `f32`.
    eq_qs: [AtomicU32; MAX_BANDS],
    /// Each band's gain in decibels, as the bits of an `f32`.
    eq_gains: [AtomicU32; MAX_BANDS],
    /// How many bands are in use.
    eq_bands: AtomicU8,
    /// Bumped whenever any of it changes, so the callback knows to look.
    pub(crate) eq_seq: AtomicU64,
    /// Length of the loaded track in milliseconds.
    pub(crate) duration_ms: AtomicU64,
    /// Set by the decoder when no more samples are coming.
    pub(crate) ended: AtomicBool,
    /// Whether a track is loaded at all.
    pub(crate) loaded: AtomicBool,
    /// Whether a following track is open and waiting to be joined on.
    pub(crate) armed: AtomicBool,
    /// A join the decoder has made and nobody has announced yet.
    ///
    /// Recorded when it happens rather than worked out afterwards. The
    /// state it would have to be worked out from — what is armed, whether a
    /// fade is running — is rebuilt by the queue four times a second, so a
    /// moment after a swap it says the opposite of what is true.
    pub(crate) joined: AtomicBool,
    /// A copy of what actually went to the device, for the visualiser.
    ///
    /// The end of the chain rather than the middle of it: what is drawn is what
    /// is heard, equaliser, fades and volume included (PROJECT_MASTER 8.1 puts
    /// the tap last for the same reason).
    pub(crate) tap: SampleRing,
    /// Whether anybody is looking.
    ///
    /// Nothing is copied while the answer is no. Section 2.9 asks for the work
    /// to stop when the visualiser is hidden or the window is away, and the
    /// cheapest place to stop it is before it starts.
    pub(crate) tapping: AtomicBool,
    /// Bumped by the decoder to ask the callback to discard the ring.
    flush_seq: AtomicU64,
    /// Echoed by the callback once it has.
    flush_ack: AtomicU64,
    /// Position, in frames, the callback should resume counting from.
    flush_base: AtomicU64,
    /// Callbacks that ran out of samples. Diagnostics only.
    pub(crate) underruns: AtomicU64,
    /// Why decoding stopped, when it stopped badly.
    ///
    /// A mutex is fine here: the callback never touches it, and the decode
    /// thread writes it at most once per track.
    pub(crate) failure: Mutex<Option<String>>,
}

impl Shared {
    /// Builds the shared state for one output configuration.
    pub(crate) fn new(rate: u32, channels: u16) -> Self {
        Self {
            ring: SampleRing::new((rate * PREBUFFER_SECONDS) as usize, channels),
            rate,
            channels,
            playing: AtomicBool::new(false),
            primed: AtomicBool::new(false),
            prime_samples: (rate as f32 * PRIME_SECONDS) as usize * usize::from(channels.max(1)),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            frames_played: AtomicU64::new(0),
            frames_pushed: AtomicU64::new(0),
            track_base: AtomicU64::new(0),
            boundary: AtomicU64::new(NO_BOUNDARY),
            announce_at: AtomicU64::new(NO_BOUNDARY),
            boundary_duration_ms: AtomicU64::new(0),
            advances: AtomicU64::new(0),
            crossfade_ms: AtomicU64::new(DEFAULT_CROSSFADE.as_millis()),
            eq_mode: AtomicU8::new(0),
            eq_frequencies: [const { AtomicU32::new(1_000) }; MAX_BANDS],
            eq_qs: [const { AtomicU32::new(0x3F80_0000) }; MAX_BANDS],
            eq_gains: [const { AtomicU32::new(0) }; MAX_BANDS],
            eq_bands: AtomicU8::new(0),
            eq_seq: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            loaded: AtomicBool::new(false),
            armed: AtomicBool::new(false),
            joined: AtomicBool::new(false),
            tap: SampleRing::new(TAP_FRAMES, channels),
            tapping: AtomicBool::new(false),
            flush_seq: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            flush_base: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            failure: Mutex::new(None),
        }
    }

    /// Publishes an equaliser setting for the callback to walk towards.
    ///
    /// The gains go out first and the sequence number last, released: the
    /// callback tests the sequence, so by the time it sees a new one every
    /// value behind it is already in place.
    pub(crate) fn set_eq(&self, mode: EqMode, bands: &[(u32, f32, f32)]) {
        self.eq_mode.store(
            match mode {
                EqMode::Simple => 0,
                EqMode::Advanced => 1,
            },
            Ordering::Relaxed,
        );

        for band in 0..MAX_BANDS {
            let (frequency_hz, q, gain_db) = bands.get(band).copied().unwrap_or((1_000, 1.0, 0.0));
            self.eq_frequencies[band].store(frequency_hz, Ordering::Relaxed);
            self.eq_qs[band].store(q.to_bits(), Ordering::Relaxed);
            self.eq_gains[band].store(gain_db.to_bits(), Ordering::Relaxed);
        }

        self.eq_bands
            .store(bands.len().min(MAX_BANDS) as u8, Ordering::Relaxed);
        self.eq_seq.fetch_add(1, Ordering::Release);
    }

    /// How many bands are published.
    pub(crate) fn eq_band_count(&self) -> usize {
        usize::from(self.eq_bands.load(Ordering::Relaxed))
    }

    /// One band, as it was published: where it sits, how wide, how loud.
    pub(crate) fn eq_band(&self, band: usize) -> (f32, f32, f32) {
        (
            self.eq_frequencies[band].load(Ordering::Relaxed) as f32,
            f32::from_bits(self.eq_qs[band].load(Ordering::Relaxed)),
            f32::from_bits(self.eq_gains[band].load(Ordering::Relaxed)),
        )
    }

    /// Which set of controls is published.
    pub(crate) fn eq_mode(&self) -> EqMode {
        if self.eq_mode.load(Ordering::Relaxed) == 0 {
            EqMode::Simple
        } else {
            EqMode::Advanced
        }
    }

    /// Sets the output level.
    pub(crate) fn set_volume(&self, volume: Volume) {
        self.volume
            .store(amplitude(volume).to_bits(), Ordering::Relaxed);
    }

    /// Where playback has reached, in the track now playing.
    ///
    /// The difference between two counters rather than one number: output runs
    /// on without interruption across a join, and it is only the base that
    /// moves when a new track takes over.
    pub(crate) fn position(&self) -> PlaybackPosition {
        let played = self.frames_played.load(Ordering::Relaxed);
        let base = self.track_base.load(Ordering::Relaxed);
        let frames = played.saturating_sub(base);
        PlaybackPosition::from_millis(frames * 1_000 / u64::from(self.rate))
    }

    /// Records frames the decoder has handed to the ring.
    fn pushed(&self, frames: u64) {
        self.frames_pushed.fetch_add(frames, Ordering::Relaxed);
    }

    /// The frame the next sample pushed will occupy.
    fn write_head(&self) -> u64 {
        self.frames_pushed.load(Ordering::Relaxed)
    }

    /// Says where a new track starts, when to say so, and how long it runs.
    ///
    /// Called once per transition, before any of that track's samples are
    /// queued, so the callback sees the mark no later than the audio it marks.
    /// `announce_at` is stored last and released: it is the one the callback
    /// tests, so everything it will read is already in place when it passes.
    fn mark_boundary(&self, frame: u64, announce_at: u64, duration: DurationMs) {
        self.boundary.store(frame, Ordering::Relaxed);
        self.boundary_duration_ms
            .store(duration.as_millis(), Ordering::Relaxed);
        self.announce_at.store(announce_at, Ordering::Release);
    }

    /// Applies a transition the decoder has already made but nobody has heard
    /// announced yet.
    ///
    /// A gapless join swaps the tracks over inside the decoder and marks the
    /// boundary in the same breath, leaving the announcement to the callback —
    /// which is seconds behind, because that is the point of a ring buffer.
    /// Seeking in that window used to throw the mark away with the samples in
    /// front of it, and the join had *already happened*: the engine went on
    /// playing the next track while nothing above it was ever told. The player
    /// bar kept the previous track's name and cover, and the timeline kept its
    /// length while running on the new track's position (`MASTER_ISSUES` 83).
    ///
    /// So the announcement is made rather than lost. The callback's own path
    /// does the same three things at the boundary; `track_base` is left to the
    /// flush that follows, which resets it to where the seek landed.
    fn announce_pending(&self) -> bool {
        if !self.joined.load(Ordering::Relaxed)
            || self.announce_at.load(Ordering::Acquire) == NO_BOUNDARY
        {
            return false;
        }
        self.joined.store(false, Ordering::Relaxed);

        self.duration_ms.store(
            self.boundary_duration_ms.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.advances.fetch_add(1, Ordering::Relaxed);
        self.clear_boundary();
        true
    }

    /// Forgets a transition that was published but never reached.
    fn clear_boundary(&self) {
        self.announce_at.store(NO_BOUNDARY, Ordering::Relaxed);
        self.boundary.store(NO_BOUNDARY, Ordering::Relaxed);
    }

    /// Length of the loaded track.
    pub(crate) fn duration(&self) -> DurationMs {
        DurationMs::from_millis(self.duration_ms.load(Ordering::Relaxed))
    }

    /// Records why decoding stopped, or clears the last reason.
    pub(crate) fn set_failure(&self, reason: Option<String>) {
        *self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = reason;
    }

    /// Asks the callback to discard everything queued and resume counting from
    /// `position`, then waits until it has.
    ///
    /// The decoder cannot empty the ring itself: the read index belongs to the
    /// callback, and two threads moving it is the one thing the ring's safety
    /// argument rules out. Waiting for the acknowledgement matters — writing
    /// fresh samples before the drain would see them thrown away with the stale
    /// ones, and the reported position would drift by however many were lost.
    fn flush(&self, position: PlaybackPosition) {
        let frames = position.as_millis() * u64::from(self.rate) / 1_000;
        self.flush_base.store(frames, Ordering::Relaxed);
        self.primed.store(false, Ordering::Relaxed);
        // Both ends of the ruler move together, and any transition queued
        // behind the discarded samples goes with them.
        self.frames_pushed.store(frames, Ordering::Relaxed);
        self.track_base.store(0, Ordering::Relaxed);
        self.clear_boundary();

        let next = self.flush_seq.load(Ordering::Relaxed).wrapping_add(1);
        self.flush_seq.store(next, Ordering::Release);

        let deadline = Instant::now() + FLUSH_TIMEOUT;
        while self.flush_ack.load(Ordering::Acquire) != next {
            if Instant::now() >= deadline {
                // The callback is not running. Nothing is reaching the speakers
                // either, so the stale samples are harmless; carry on rather
                // than block the listener's seek on a dead device.
                self.frames_played.store(frames, Ordering::Relaxed);
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Converts a volume control value into an amplitude.
///
/// A slider is linear in position and hearing is not: halving the amplitude
/// costs 6 dB, which sounds like a small step, so a linear mapping crowds every
/// useful level into the bottom of the travel. Squaring puts the middle of the
/// slider near −12 dB, which is roughly where "half as loud" is heard.
///
/// ponytail: one squaring. A full loudness curve is a lookup table and a
/// calibration argument nobody has asked for.
fn amplitude(volume: Volume) -> f32 {
    let value = volume.as_f32();
    value * value
}

/// Fills one output buffer. **This runs on the realtime audio thread.**
///
/// Everything it does is in section 8.2's allowed list: atomic reads, a
/// lock-free ring, and multiplication. `gain` is the callback's own state, kept
/// across calls so that a ramp survives the buffer boundary.
pub(crate) fn fill_output(shared: &Shared, out: &mut [f32], gain: &mut f32, eq: &mut EqChain) {
    // A flush was asked for: discard the queue, adopt the new position, and come
    // back from silence so the discontinuity cannot be heard.
    let seq = shared.flush_seq.load(Ordering::Acquire);
    if seq != shared.flush_ack.load(Ordering::Relaxed) {
        shared.ring.drain();
        shared
            .frames_played
            .store(shared.flush_base.load(Ordering::Relaxed), Ordering::Relaxed);
        *gain = 0.0;
        shared.flush_ack.store(seq, Ordering::Release);
    }

    // Nothing has been decoded yet. Silence now is not an underrun: the device
    // simply started before the decoder did, which happens on every track.
    if !shared.primed.load(Ordering::Relaxed) {
        if shared.ring.available() < shared.prime_samples && !shared.ended.load(Ordering::Relaxed) {
            out.fill(0.0);
            return;
        }
        shared.primed.store(true, Ordering::Relaxed);
    }

    let target = if shared.playing.load(Ordering::Relaxed) {
        f32::from_bits(shared.volume.load(Ordering::Relaxed))
    } else {
        0.0
    };

    // Paused and already silent: leave the queue alone. Consuming here is what
    // would make a pause lose the samples it is holding.
    if target == 0.0 && *gain == 0.0 {
        out.fill(0.0);
        return;
    }

    let taken = shared.ring.pop(out);
    out[taken..].fill(0.0);

    if taken < out.len() && !shared.ended.load(Ordering::Relaxed) {
        shared.underruns.fetch_add(1, Ordering::Relaxed);
    }

    // Section 8.1 puts the equaliser before the stream's own volume, and so
    // does this: the listener's gain is the last thing applied, so a boosted
    // band is turned down by the slider like everything else.
    eq.process(shared, out);

    let step = 1.0 / (RAMP_SECONDS * shared.rate as f32);
    for frame in out.chunks_mut(usize::from(shared.channels)) {
        *gain = if *gain < target {
            (*gain + step).min(target)
        } else {
            (*gain - step).max(target)
        };
        for sample in frame {
            *sample *= *gain;
        }
    }

    // The tap, last of all: what is copied is exactly what left, after the
    // equaliser and after the gain. Gated, because nothing should be paid for
    // while nobody is looking — and it is a copy of a buffer, which is the one
    // thing section 8.2 allows here.
    if shared.tapping.load(Ordering::Relaxed) {
        shared.tap.push(out);
    }

    let frames = taken / usize::from(shared.channels);
    let played = shared
        .frames_played
        .fetch_add(frames as u64, Ordering::Relaxed)
        + frames as u64;

    // Output has reached the point where the join should be said out loud. The
    // audio needed nothing from this — it was mixed into the samples that just
    // played — but the clock does: from here the position belongs to the new
    // track, counted from where that track actually began.
    let announce_at = shared.announce_at.load(Ordering::Acquire);
    if announce_at != NO_BOUNDARY && played >= announce_at {
        shared
            .track_base
            .store(shared.boundary.load(Ordering::Relaxed), Ordering::Relaxed);
        shared.duration_ms.store(
            shared.boundary_duration_ms.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        shared.announce_at.store(NO_BOUNDARY, Ordering::Relaxed);
        shared.joined.store(false, Ordering::Relaxed);
        shared.advances.fetch_add(1, Ordering::Relaxed);
    }
}

/// What the control thread asks the decode thread to do.
pub(crate) enum Command {
    /// Replace the current track.
    Load {
        /// File to play.
        path: PathBuf,
        /// Where the outcome goes, so `load` can report a real error.
        reply: Sender<Result<StreamInfo>>,
    },
    /// Open the track that follows, so the join can be decoded before it.
    Preload {
        /// File to play after the current one.
        path: PathBuf,
        /// How the two should meet.
        transition: TransitionProfile,
        /// Where the outcome goes: a file that will not open is worth knowing
        /// about seconds early rather than as a silence.
        reply: Sender<Result<()>>,
    },
    /// Jump within the current track.
    Seek {
        /// Requested position.
        position: PlaybackPosition,
        /// Where the position actually reached goes.
        reply: Sender<Result<PlaybackPosition>>,
    },
    /// Unload the current track.
    Stop,
    /// Finish, so the thread can be joined.
    Shutdown,
}

/// The decode thread.
///
/// Runs until it is told to shut down or the engine is dropped.
pub(crate) fn decode_loop(shared: Arc<Shared>, commands: &Receiver<Command>) {
    let mut producer = Producer::new(shared);

    loop {
        let command = if producer.is_idle() {
            // Nothing to decode: block rather than spin. A disconnected channel
            // means the engine is gone.
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        } else {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };

        match command {
            Some(Command::Shutdown) => return,
            Some(command) => producer.handle(command),
            // Nothing waiting: decode a little more, and wait if the ring is
            // full rather than burn a core spinning on it.
            None => {
                if !producer.pump() {
                    thread::sleep(IDLE_NAP);
                }
            }
        }
    }
}

/// One source being decoded: the file, its resampler, and what it has produced
/// but not yet handed over.
///
/// Two of these are alive through every transition, which is the whole of what
/// makes gapless and crossfade possible (ADR 5). They are peers: nothing here
/// knows which one is playing.
struct Lane {
    source: TrackStream,
    resampler: Option<Resampling>,
    /// Samples at the output rate and layout, waiting to be taken.
    ready: Vec<f32>,
    /// How much of `ready` has already been taken.
    taken: usize,
    /// Channel-mapped frames, kept between passes to avoid reallocating.
    mapped: Vec<f32>,
    /// Output frames produced so far.
    frames: u64,
    /// The whole length in output frames, or zero where the container will not
    /// say. Only a crossfade needs it: a gapless join is made where the file
    /// actually ends, not where it was advertised to.
    total: u64,
    /// Set once the decoder has nothing left to give.
    drained: bool,
    /// Why it stopped, when it stopped badly.
    failure: Option<String>,
}

impl Lane {
    fn open(path: &Path, rate: u32, channels: u16) -> Result<Self> {
        let source = TrackStream::open(path)?;
        let info = source.info();

        let resampler = if info.sample_rate == rate {
            None
        } else {
            Some(Resampling::new(info.sample_rate, rate, channels)?)
        };

        Ok(Self {
            source,
            resampler,
            ready: Vec::new(),
            taken: 0,
            mapped: Vec::new(),
            frames: 0,
            total: info.duration.as_millis() * u64::from(rate) / 1_000,
            drained: false,
            failure: None,
        })
    }

    fn info(&self) -> StreamInfo {
        self.source.info()
    }

    /// Samples decoded and not yet taken.
    fn ready(&self) -> &[f32] {
        &self.ready[self.taken..]
    }

    /// Decodes until `want` samples are waiting, or the file runs out.
    fn fill(&mut self, want: usize, channels: u16) {
        if self.taken > 0 {
            self.ready.drain(..self.taken);
            self.taken = 0;
        }
        while self.ready.len() < want && !self.drained {
            self.decode_once(channels);
        }
    }

    fn decode_once(&mut self, channels: u16) {
        let source_channels = self.info().channels;

        // The mapping happens inside the match so the decoder's borrow ends
        // with it: what comes back is a view into the lane's own reader.
        let outcome = match self.source.next_frames() {
            Ok(Some(frames)) => {
                map_channels(frames, source_channels, channels, &mut self.mapped);
                Ok(())
            }
            Ok(None) => Err(None),
            Err(err) => Err(Some(err.to_string())),
        };

        if let Err(failure) = outcome {
            self.failure = failure;
            self.drained = true;
            return;
        }

        match self.resampler.as_mut() {
            Some(resampler) => match resampler.process(&self.mapped) {
                Ok(resampled) => self.ready.extend_from_slice(resampled),
                Err(err) => {
                    self.failure = Some(err.to_string());
                    self.drained = true;
                }
            },
            None => self.ready.extend_from_slice(&self.mapped),
        }
    }

    /// Marks samples as used, and counts the frames they were.
    fn consume(&mut self, samples: usize, channels: usize) {
        self.taken += samples;
        self.frames += (samples / channels) as u64;
    }

    fn seek(&mut self, position: PlaybackPosition, rate: u32) -> Result<PlaybackPosition> {
        let landed = self.source.seek(position)?;

        self.ready.clear();
        self.taken = 0;
        self.drained = false;
        self.frames = landed.as_millis() * u64::from(rate) / 1_000;
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }

        Ok(landed)
    }
}

/// The decode thread's own state.
struct Producer {
    shared: Arc<Shared>,
    /// The track being played.
    current: Option<Lane>,
    /// The track armed to follow it.
    next: Option<Lane>,
    /// How the two are to meet.
    transition: TransitionProfile,
    /// Mixed samples not yet accepted by the ring.
    out: Vec<f32>,
    /// How much of `out` has been handed over.
    out_taken: usize,
    /// Length of the crossfade under way, in frames. Zero when none is.
    fade_frames: u64,
    /// How much of it has been mixed.
    fade_done: u64,
}

impl Producer {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            current: None,
            next: None,
            transition: TransitionProfile::Gapless,
            out: Vec::new(),
            out_taken: 0,
            fade_frames: 0,
            fade_done: 0,
        }
    }

    /// True when there is nothing left to decode or hand over.
    ///
    /// A finished track counts as idle even though its file is still open: it is
    /// held so that the listener can seek back into it. A finished track with
    /// something armed behind it does not — the join is still to be made.
    ///
    /// Drained is not finished. The decoder stops several ring-lengths before
    /// the listener does, and the track is only over once [`Self::produce`] has
    /// said so by setting `ended`. Going to sleep in between leaves that flag
    /// unset for good — the loop blocks on the next command, `produce` is never
    /// reached again, and the engine reports a track that is playing in silence
    /// for ever (MASTER_ISSUES 46).
    fn is_idle(&self) -> bool {
        if self.out_taken < self.out.len() || self.next.is_some() {
            return false;
        }

        let Some(lane) = self.current.as_ref() else {
            return true;
        };
        lane.drained && lane.ready().is_empty() && self.shared.ended.load(Ordering::Relaxed)
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Load { path, reply } => {
                let outcome = self.load(&path);
                let _ = reply.send(outcome);
            }
            Command::Preload {
                path,
                transition,
                reply,
            } => {
                let outcome = self.preload(&path, transition);
                let _ = reply.send(outcome);
            }
            Command::Seek { position, reply } => {
                let outcome = self.seek(position);
                let _ = reply.send(outcome);
            }
            Command::Stop => self.unload(),
            Command::Shutdown => unreachable!("handled by the loop"),
        }
    }

    fn load(&mut self, path: &Path) -> Result<StreamInfo> {
        let lane = Lane::open(path, self.shared.rate, self.shared.channels)?;
        let info = lane.info();

        // A hard load replaces everything, the armed track included: whatever
        // was going to follow was going to follow something else.
        self.current = Some(lane);
        self.disarm();
        self.out.clear();
        self.out_taken = 0;

        self.shared.set_failure(None);
        self.shared
            .duration_ms
            .store(info.duration.as_millis(), Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.loaded.store(true, Ordering::Relaxed);
        self.shared.flush(PlaybackPosition::START);

        Ok(info)
    }

    /// Opens the track that follows and says how it should arrive.
    ///
    /// Opening it here, on the decode thread, is the point: by the time the
    /// join is reached the file has been read, its decoder built and its first
    /// packets turned into samples, so nothing about the handover waits on a
    /// disk.
    fn preload(&mut self, path: &Path, transition: TransitionProfile) -> Result<()> {
        if self.current.is_none() {
            return Err(CoreError::Audio(
                "nothing is playing for a track to follow".into(),
            ));
        }

        let lane = Lane::open(path, self.shared.rate, self.shared.channels)?;
        self.next = Some(lane);
        self.transition = transition;
        self.shared.armed.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn seek(&mut self, position: PlaybackPosition) -> Result<PlaybackPosition> {
        let rate = self.shared.rate;
        let Some(current) = self.current.as_mut() else {
            return Err(CoreError::Audio("nothing is loaded to seek in".into()));
        };

        let landed = current.seek(position, rate)?;

        // A join the decoder has already made and the callback has not yet
        // reached. Announced here rather than thrown away with the samples
        // in front of it: the swap has happened, and the track being seeked
        // is no longer the one the rest of the application believes is
        // playing.
        self.shared.announce_pending();

        // Whatever was armed had already begun to be mixed in at a point that
        // no longer exists. It is dropped rather than rewound, and the caller
        // arms it again — which it does anyway, on every tick.
        self.disarm();
        self.out.clear();
        self.out_taken = 0;

        // The track may have run out earlier and been seeked back into.
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.flush(landed);

        Ok(landed)
    }

    fn unload(&mut self) {
        self.current = None;
        self.disarm();
        self.out.clear();
        self.out_taken = 0;

        self.shared.loaded.store(false, Ordering::Relaxed);
        self.shared.ended.store(true, Ordering::Relaxed);
        self.shared.duration_ms.store(0, Ordering::Relaxed);
        self.shared.flush(PlaybackPosition::START);
    }

    /// Forgets the armed track and any fade that had begun on it.
    ///
    /// The record of a join goes too. Seeking announces one before calling
    /// this, so nothing is lost — and leaving the flag set would let a later
    /// seek announce a crossfade's mark as though the swap behind it had
    /// happened, when mid-fade it has not.
    fn disarm(&mut self) {
        self.next = None;
        self.fade_frames = 0;
        self.fade_done = 0;
        self.shared.armed.store(false, Ordering::Relaxed);
        self.shared.joined.store(false, Ordering::Relaxed);
    }

    /// Moves one step of work along. Returns false when there was nothing to do.
    fn pump(&mut self) -> bool {
        if self.out_taken < self.out.len() {
            let accepted = self.shared.ring.push(&self.out[self.out_taken..]);
            self.out_taken += accepted;
            self.shared
                .pushed((accepted / usize::from(self.shared.channels)) as u64);
            return accepted > 0;
        }

        if self.current.is_none() {
            return false;
        }

        // Nothing more is coming and nothing is waiting to follow.
        if self.shared.ended.load(Ordering::Relaxed) && self.next.is_none() {
            return false;
        }

        // Do not decode further ahead than the ring can hold.
        if self.shared.ring.available() == self.shared.ring.capacity() {
            return false;
        }

        self.produce()
    }

    /// Mixes the next block into `out`. Returns false when there was none to mix.
    fn produce(&mut self) -> bool {
        let channels = usize::from(self.shared.channels);
        let want = BLOCK_SAMPLES / channels * channels;

        if self.fade_frames == 0 {
            self.begin_fade_if_due();
        }
        if self.fade_frames > 0 {
            return self.mix_fade(want, channels);
        }

        // Stop the block exactly where the fade will begin. Left to run its
        // full length it would overshoot, the fade would start at whatever
        // boundary first fell inside it, and a four-second crossfade would come
        // out short by an amount that depended on the buffer size.
        let want = match self.frames_until_fade() {
            Some(frames) if frames > 0 => want.min(frames as usize * channels),
            _ => want,
        };

        let Some(current) = self.current.as_mut() else {
            return false;
        };
        current.fill(want, self.shared.channels);

        let available = current.ready().len();
        if available == 0 {
            if !current.drained {
                return false;
            }
            // The file has run out. Either the armed track takes over from this
            // exact sample, or there is nothing more to play.
            let failure = current.failure.take();
            if self.next.is_some() {
                self.swap_in_next(true);
                return true;
            }
            self.finish(failure);
            return false;
        }

        let taking = available.min(want);
        self.out.clear();
        self.out.extend_from_slice(&current.ready()[..taking]);
        current.consume(taking, channels);
        self.out_taken = 0;
        true
    }

    /// How many frames of the outgoing track are left before the fade starts.
    ///
    /// `None` where no crossfade is coming: nothing armed, a gapless join, or a
    /// container that will not say how long it is. That last one cannot be
    /// faded out on a schedule — there is nothing to count back from — so it
    /// joins gapless instead, which is the honest fallback rather than a silent
    /// one.
    fn frames_until_fade(&self) -> Option<u64> {
        if self.transition != TransitionProfile::Crossfade || self.next.is_none() {
            return None;
        }

        let current = self.current.as_ref()?;
        if current.total == 0 {
            return None;
        }

        let remaining = current.total.saturating_sub(current.frames);
        Some(remaining.saturating_sub(self.fade_wanted(current.total)))
    }

    /// How long a fade over the outgoing track would run.
    ///
    /// Asked of the whole track rather than of what is left of it, so that the
    /// number does not move as the track plays: the same one decides when the
    /// fade starts and how long it then lasts.
    fn fade_wanted(&self, total: u64) -> u64 {
        let stored =
            self.shared.crossfade_ms.load(Ordering::Relaxed) * u64::from(self.shared.rate) / 1_000;
        fade_length(stored, total)
    }

    /// Starts the crossfade once the outgoing track has only its length left.
    fn begin_fade_if_due(&mut self) {
        if self.frames_until_fade() != Some(0) {
            return;
        }

        let current = self.current.as_ref().expect("checked above");
        let remaining = current.total.saturating_sub(current.frames);

        // The `min` is for a track armed later than it should have been: there
        // is no fading four seconds of a track with one second left.
        self.fade_frames = self.fade_wanted(current.total).min(remaining).max(1);
        self.fade_done = 0;

        // The new track is heard from here, so this is where its clock starts —
        // not where the old one finally stops. It is announced half a fade
        // later, at the point where it becomes the louder of the two.
        let head = self.shared.write_head();
        let duration = self.next.as_ref().expect("armed above").info().duration;
        self.shared
            .mark_boundary(head, head + self.fade_frames / 2, duration);
    }

    /// Mixes both lanes for one block of the crossfade.
    fn mix_fade(&mut self, want: usize, channels: usize) -> bool {
        let (fade_frames, fade_done) = (self.fade_frames, self.fade_done);
        let layout = self.shared.channels;

        let (Some(current), Some(next)) = (self.current.as_mut(), self.next.as_mut()) else {
            self.fade_frames = 0;
            return false;
        };

        current.fill(want, layout);
        next.fill(want, layout);

        let left = ((fade_frames - fade_done) as usize) * channels;
        let head = next.ready().len().min(want).min(left);
        if head == 0 {
            if !next.drained {
                return false;
            }
            // The incoming track is shorter than the fade. Stop fading and let
            // it take over as it is; the ordinary path will find it finished.
            self.swap_in_next(false);
            return true;
        }

        // A tail shorter than the head is normal at the end of a file: the
        // outgoing track simply contributes silence for the rest of the fade,
        // and the incoming one still arrives at full level on schedule.
        let tail = current.ready().len().min(head);
        let frames = head / channels;

        self.out.clear();
        self.out.reserve(head);
        for frame in 0..frames {
            let t = (fade_done + frame as u64) as f32 / fade_frames as f32;
            let (fade_out, fade_in) = equal_power(t);
            for channel in 0..channels {
                let index = frame * channels + channel;
                let outgoing = if index < tail {
                    current.ready()[index] * fade_out
                } else {
                    0.0
                };
                self.out.push(outgoing + next.ready()[index] * fade_in);
            }
        }

        current.consume(tail, channels);
        next.consume(head, channels);
        self.out_taken = 0;
        self.fade_done += frames as u64;

        if self.fade_done >= self.fade_frames {
            // The boundary was published when the fade began, which is when the
            // new track was first heard.
            self.swap_in_next(false);
        }
        true
    }

    /// Makes the armed track the current one.
    ///
    /// `mark` says whether the join still has to be published. It is false at
    /// the end of a crossfade, where the mark went out when the fade started.
    fn swap_in_next(&mut self, mark: bool) {
        let Some(next) = self.next.take() else {
            return;
        };

        if mark {
            // Published before a single sample of the new track is queued: the
            // callback must never meet audio it has no mark for.
            let head = self.shared.write_head();
            self.shared.mark_boundary(head, head, next.info().duration);
        }

        self.current = Some(next);
        self.fade_frames = 0;
        self.fade_done = 0;
        self.shared.armed.store(false, Ordering::Relaxed);
        self.shared.joined.store(true, Ordering::Relaxed);
    }

    /// Marks the end of the stream, with a reason when it ended badly.
    ///
    /// The file stays open and the ring is left alone. Both matter: what is
    /// queued has already been decoded and is still owed to the listener, and
    /// the decoder runs seconds ahead of the speakers — so by the time the last
    /// note is heard the source has long since hit the end of the file. Dropping
    /// it there would make the final seconds of every track unseekable.
    fn finish(&mut self, failure: Option<String>) {
        if failure.is_some() {
            self.shared.set_failure(failure);
        }
        self.shared.ended.store(true, Ordering::Relaxed);
    }
}

/// Rearranges interleaved frames from the file's channel layout to the device's.
///
/// ponytail: mono is duplicated, a matching layout passes through, and anything
/// wider is truncated to the first channels. A 5.1 file therefore loses its
/// centre and surrounds rather than being folded down. Proper downmix
/// coefficients are a table and a listening test; the material section 2.2
/// describes is overwhelmingly stereo, and the upgrade is local to this
/// function.
fn map_channels(input: &[f32], from: u16, to: u16, out: &mut Vec<f32>) {
    out.clear();

    if from == to {
        out.extend_from_slice(input);
        return;
    }

    let from = usize::from(from).max(1);
    let to = usize::from(to).max(1);

    for frame in input.chunks_exact(from) {
        if from == 1 {
            // Mono into every output channel, rather than into the left one
            // alone — which is what "one speaker is broken" sounds like.
            out.extend(std::iter::repeat_n(frame[0], to));
        } else if to == 1 {
            let sum: f32 = frame.iter().sum();
            out.push(sum / frame.len() as f32);
        } else {
            let shared = from.min(to);
            out.extend_from_slice(&frame[..shared]);
            out.extend(std::iter::repeat_n(0.0, to.saturating_sub(shared)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use cadenza_core::domain::playback::TransitionProfile;
    use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Volume};
    use cadenza_testkit::TempDir;
    use cadenza_testkit::audio_fixtures::write_wav;

    use super::{Producer, Shared, amplitude, map_channels};
    use crate::audio::eq::EqChain;

    /// The callback, with an equaliser that is flat and so skips itself.
    ///
    /// A wrapper because the chain is the callback's own state, like its gain:
    /// what these tests are about is the ring and the clock, and a flat chain
    /// returns before it touches a sample.
    fn fill_output(shared: &Shared, out: &mut [f32], gain: &mut f32) {
        let mut eq = EqChain::new(shared.rate, shared.channels);
        super::fill_output(shared, out, gain, &mut eq);
    }

    /// A stereo engine at a rate low enough that a five-millisecond ramp is a
    /// handful of frames, so the tests can assert on it inside one buffer.
    fn shared() -> Shared {
        Shared::new(1_000, 2)
    }

    /// Puts a stream in the state it reaches once the decoder has caught up.
    fn ready(shared: &Shared) {
        shared.playing.store(true, Ordering::Relaxed);
        shared.primed.store(true, Ordering::Relaxed);
        shared.set_volume(Volume::FULL);
    }

    #[test]
    fn a_paused_stream_outputs_silence_and_keeps_its_samples() {
        let shared = shared();
        shared.ring.push(&[1.0; 64]);

        let mut out = [1.0; 8];
        let mut gain = 0.0;
        fill_output(&shared, &mut out, &mut gain);

        assert_eq!(out, [0.0; 8]);
        assert_eq!(shared.ring.available(), 64, "nothing was consumed");
        assert_eq!(shared.position(), PlaybackPosition::START);
    }

    #[test]
    fn nothing_is_consumed_until_enough_has_been_decoded() {
        let shared = shared();
        shared.playing.store(true, Ordering::Relaxed);
        shared.set_volume(Volume::FULL);
        shared.ring.push(&[1.0; 8]);

        let mut out = [9.0; 8];
        let mut gain = 1.0;
        fill_output(&shared, &mut out, &mut gain);

        assert_eq!(out, [0.0; 8], "the device waits rather than stuttering");
        assert_eq!(
            shared.ring.available(),
            8,
            "and the samples are still there"
        );
        assert_eq!(
            shared.underruns.load(Ordering::Relaxed),
            0,
            "starting before the decoder is not an underrun"
        );
    }

    #[test]
    fn a_stream_that_ended_short_plays_out_rather_than_waiting_forever() {
        let shared = shared();
        shared.playing.store(true, Ordering::Relaxed);
        shared.set_volume(Volume::FULL);
        shared.ended.store(true, Ordering::Relaxed);
        shared.ring.push(&[1.0; 8]);

        let mut out = [0.0; 8];
        let mut gain = 1.0;
        fill_output(&shared, &mut out, &mut gain);

        assert!(shared.ring.is_empty(), "a file shorter than the prebuffer");
    }

    #[test]
    fn playing_consumes_the_ring_and_advances_the_position() {
        let shared = shared();
        ready(&shared);
        shared.ring.push(&[1.0; 200]);

        let mut out = [0.0; 200];
        let mut gain = 1.0;
        fill_output(&shared, &mut out, &mut gain);

        assert!(shared.ring.is_empty());
        assert_eq!(
            shared.position(),
            PlaybackPosition::from_millis(100),
            "100 stereo frames at 1 kHz is 100 ms"
        );
    }

    #[test]
    fn an_underrun_is_silence_rather_than_a_repeat_of_the_last_buffer() {
        let shared = shared();
        ready(&shared);
        shared.ring.push(&[0.5; 4]);

        let mut out = [9.0; 16];
        let mut gain = 1.0;
        fill_output(&shared, &mut out, &mut gain);

        assert_eq!(&out[4..], &[0.0; 12], "the tail is silent");
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_finished_track_running_dry_is_not_counted_as_an_underrun() {
        let shared = shared();
        ready(&shared);
        shared.ended.store(true, Ordering::Relaxed);

        let mut gain = 1.0;
        fill_output(&shared, &mut [0.0; 16], &mut gain);

        assert_eq!(shared.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_flush_throws_away_the_queue_and_moves_the_position() {
        let shared = shared();
        shared.playing.store(true, Ordering::Relaxed);
        shared.ring.push(&[1.0; 64]);

        // No callback is running, so the flush gives up on the acknowledgement
        // and returns; the next callback still performs the drain.
        shared.flush(PlaybackPosition::from_millis(500));

        let mut gain = 1.0;
        fill_output(&shared, &mut [0.0; 8], &mut gain);

        assert!(shared.ring.is_empty(), "the stale samples were dropped");
        assert_eq!(shared.position(), PlaybackPosition::from_millis(500));
    }

    #[test]
    fn gain_moves_towards_its_target_instead_of_jumping() {
        let shared = shared();
        ready(&shared);
        shared.ring.push(&[1.0; 8]);

        let mut out = [0.0; 8];
        let mut gain = 0.0;
        fill_output(&shared, &mut out, &mut gain);

        // At 1 kHz a five-millisecond ramp is five frames, so the buffer starts
        // near silence and climbs.
        assert!(out[0] < out[3], "the buffer ramps up: {out:?}");
        assert!(out[0] < 0.5, "and it starts near silence");
    }

    #[test]
    fn the_volume_taper_is_quieter_than_the_slider_position() {
        assert!((amplitude(Volume::FULL) - 1.0).abs() < f32::EPSILON);
        assert_eq!(amplitude(Volume::MUTED), 0.0);

        let half = amplitude(Volume::new(0.5).expect("in range"));
        assert!(
            (half - 0.25).abs() < f32::EPSILON,
            "half travel is a quarter of the amplitude, about -12 dB"
        );
    }

    /// Decodes a whole file with a fake consumer emptying the ring as fast as it
    /// fills, which is what the callback would be doing.
    fn decode_to_the_end(producer: &mut Producer, shared: &Shared) {
        for _ in 0..100_000 {
            producer.pump();
            shared.ring.drain();
            if shared.ended.load(Ordering::Relaxed) {
                return;
            }
        }
        panic!("the decoder never reached the end of the file");
    }

    #[test]
    fn a_finished_track_can_still_be_seeked_back_into() {
        let directory = TempDir::new("stream-seek-after-end");
        let path = write_wav(directory.path(), "tone.wav", 1, 8_000);

        let shared = Arc::new(Shared::new(44_100, 2));
        let mut producer = Producer::new(Arc::clone(&shared));
        producer.load(&path).expect("loaded");

        decode_to_the_end(&mut producer, &shared);

        // The decoder runs seconds ahead of the speakers, so this is the normal
        // state during the last part of every track — not an edge case.
        let landed = producer
            .seek(PlaybackPosition::from_millis(500))
            .expect("a finished file is still open for seeking");

        // At or a little before what was asked for: a container seeks to a
        // packet boundary, and the reported position is the real one.
        assert!(
            landed.as_millis() <= 500 && landed.as_millis() > 450,
            "landed at {landed} rather than just before half a second"
        );
        assert!(
            !shared.ended.load(Ordering::Relaxed),
            "seeking back into a track un-ends it"
        );
    }

    #[test]
    fn stopping_closes_the_file() {
        let directory = TempDir::new("stream-stop");
        let path = write_wav(directory.path(), "tone.wav", 1, 8_000);

        let shared = Arc::new(Shared::new(44_100, 2));
        let mut producer = Producer::new(Arc::clone(&shared));
        producer.load(&path).expect("loaded");
        producer.unload();

        assert!(!shared.loaded.load(Ordering::Relaxed));
        assert!(
            producer.seek(PlaybackPosition::START).is_err(),
            "there is nothing to seek in once the track is unloaded"
        );
    }

    /// Runs the decoder until it has nothing left, collecting everything it
    /// produced in order — which is what the device would have heard.
    fn decode_stream(producer: &mut Producer, shared: &Shared) -> Vec<f32> {
        let mut heard = Vec::new();
        let mut buffer = vec![0.0; 8_192];

        for _ in 0..1_000_000 {
            let worked = producer.pump();
            loop {
                let taken = shared.ring.pop(&mut buffer);
                if taken == 0 {
                    break;
                }
                heard.extend_from_slice(&buffer[..taken]);
            }
            if !worked && shared.ended.load(Ordering::Relaxed) {
                return heard;
            }
        }
        panic!("the decoder never reached the end of the join");
    }

    /// The largest step between neighbouring samples.
    ///
    /// A click is a discontinuity in the waveform, and this is the objective
    /// half of "no clicks": whatever it sounds like, a join that steps is
    /// wrong, and one that does not step cannot click at the join.
    fn largest_step(samples: &[f32]) -> f32 {
        samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max)
    }

    /// A producer at the fixtures' own rate, so nothing is resampled.
    fn joined() -> (Arc<Shared>, Producer) {
        let shared = Arc::new(Shared::new(44_100, 2));
        let producer = Producer::new(Arc::clone(&shared));
        (shared, producer)
    }

    #[test]
    fn a_gapless_join_runs_one_track_straight_into_the_next() {
        let directory = TempDir::new("stream-gapless");
        let first = write_wav(directory.path(), "first.wav", 1, 12_000);
        let second = write_wav(directory.path(), "second.wav", 1, -12_000);

        let (shared, mut producer) = joined();
        producer.load(&first).expect("loaded");
        producer
            .preload(&second, TransitionProfile::Gapless)
            .expect("armed");

        let heard = decode_stream(&mut producer, &shared);

        // Both tracks in full, and not one sample of silence anywhere in them:
        // a gap would show up here as a run of zeros.
        assert_eq!(heard.len(), 44_100 * 2 * 2, "two whole seconds, in stereo");
        assert!(
            heard.iter().all(|sample| sample.abs() > 0.3),
            "something in the join was silent"
        );

        // The second file is the first one inverted, so the join is the single
        // place the sign changes — and it is where the mark says it is.
        let join = heard
            .iter()
            .position(|sample| *sample < 0.0)
            .expect("the second track was heard");
        assert_eq!(join, 44_100 * 2, "the first track played out whole");
        assert_eq!(
            shared.boundary.load(Ordering::Relaxed),
            44_100,
            "the mark stands at the first frame of the new track"
        );
    }

    #[test]
    fn a_crossfade_arrives_on_the_equal_power_curve() {
        let directory = TempDir::new("stream-crossfade");
        let first = write_wav(directory.path(), "first.wav", 2, 12_000);
        let second = write_wav(directory.path(), "second.wav", 2, 24_000);

        let (shared, mut producer) = joined();
        // A second rather than the four the listener gets. The port refuses
        // anything outside three to five seconds; the decoder only obeys, and
        // a shorter fade is the same arithmetic over less audio.
        shared.crossfade_ms.store(1_000, Ordering::Relaxed);

        producer.load(&first).expect("loaded");
        producer
            .preload(&second, TransitionProfile::Crossfade)
            .expect("armed");

        let heard = decode_stream(&mut producer, &shared);

        // Four seconds of material, one second of which is heard twice over.
        assert_eq!(heard.len(), 44_100 * 3 * 2, "the fade overlaps the two");
        assert_eq!(
            shared.boundary.load(Ordering::Relaxed),
            44_100,
            "the new track's clock starts where it is first heard"
        );

        let quiet = 12_000.0 / 32_768.0;
        let loud = 24_000.0 / 32_768.0;
        // One second in: the fade is the last second of a two-second track.
        let fade_start = 44_100;

        for (frame, position) in [(0, 0.0), (11_025, 0.25), (22_050, 0.5), (44_099, 1.0)] {
            let angle = position * std::f32::consts::FRAC_PI_2;
            let expected = quiet * angle.cos() + loud * angle.sin();
            let actual = heard[(fade_start + frame) * 2];
            assert!(
                (actual - expected).abs() < 0.01,
                "at {position} of the way through the fade: {actual} rather than {expected}"
            );
        }

        // Nothing steps: the fade begins at exactly the outgoing level and ends
        // at exactly the incoming one, so there is no edge to hear.
        assert!(
            largest_step(&heard) < 0.001,
            "the waveform jumps by {}",
            largest_step(&heard)
        );
    }

    #[test]
    fn a_crossfade_takes_at_most_half_the_track_it_is_leaving() {
        let directory = TempDir::new("stream-short-crossfade");
        let first = write_wav(directory.path(), "first.wav", 1, 12_000);
        let second = write_wav(directory.path(), "second.wav", 1, 24_000);

        let (shared, mut producer) = joined();
        // Four seconds of fade asked of a track one second long.
        shared.crossfade_ms.store(4_000, Ordering::Relaxed);

        producer.load(&first).expect("loaded");
        producer
            .preload(&second, TransitionProfile::Crossfade)
            .expect("armed");

        let heard = decode_stream(&mut producer, &shared);

        // Half a second of fade, not four: a track this short would otherwise
        // spend all of itself underneath the next one. Two seconds of material
        // overlapping by half of one, in stereo.
        assert_eq!(heard.len(), (44_100 * 2 - 22_050) * 2);
        assert_eq!(
            shared.boundary.load(Ordering::Relaxed),
            22_050,
            "the fade begins halfway through the first track"
        );
        assert!(
            (heard[0] - 12_000.0 / 32_768.0).abs() < 0.01,
            "it still begins at the outgoing level"
        );
        assert!(largest_step(&heard) < 0.001, "and it still does not step");
    }

    #[test]
    fn loading_a_track_throws_away_whatever_was_armed_behind_the_old_one() {
        let directory = TempDir::new("stream-rearm");
        let first = write_wav(directory.path(), "first.wav", 1, 12_000);
        let second = write_wav(directory.path(), "second.wav", 1, -12_000);

        let (shared, mut producer) = joined();
        producer.load(&first).expect("loaded");
        producer
            .preload(&second, TransitionProfile::Gapless)
            .expect("armed");
        assert!(shared.armed.load(Ordering::Relaxed));

        producer.load(&second).expect("loaded");

        assert!(
            !shared.armed.load(Ordering::Relaxed),
            "what was going to follow was going to follow something else"
        );
    }

    #[test]
    fn seeking_drops_the_join_it_had_already_begun_to_mix() {
        let directory = TempDir::new("stream-seek-armed");
        let first = write_wav(directory.path(), "first.wav", 1, 12_000);
        let second = write_wav(directory.path(), "second.wav", 1, -12_000);

        let (shared, mut producer) = joined();
        producer.load(&first).expect("loaded");
        producer
            .preload(&second, TransitionProfile::Gapless)
            .expect("armed");
        producer
            .seek(PlaybackPosition::from_millis(200))
            .expect("seeked");

        assert!(
            !shared.armed.load(Ordering::Relaxed),
            "the join was aimed at a point that no longer exists"
        );
        assert_eq!(
            shared.boundary.load(Ordering::Relaxed),
            u64::MAX,
            "and the mark went with it"
        );
    }

    #[test]
    fn nothing_can_be_armed_behind_a_track_that_is_not_playing() {
        let directory = TempDir::new("stream-arm-nothing");
        let path = write_wav(directory.path(), "one.wav", 1, 12_000);

        let (_shared, mut producer) = joined();

        assert!(producer.preload(&path, TransitionProfile::Gapless).is_err());
    }

    #[test]
    fn the_callback_moves_the_clock_to_the_new_track_at_the_join() {
        let shared = shared();
        ready(&shared);
        shared.ring.push(&[1.0; 400]);
        // The decoder queued 100 frames of the old track and then the new one,
        // which runs for two seconds. A gapless join, so it is said at the
        // moment it happens.
        shared.mark_boundary(100, 100, DurationMs::from_secs(2));

        let mut gain = 1.0;
        fill_output(&shared, &mut [0.0; 160], &mut gain);
        assert_eq!(
            shared.position(),
            PlaybackPosition::from_millis(80),
            "still the old track, eighty frames in at 1 kHz"
        );
        assert_eq!(shared.advances.load(Ordering::Relaxed), 0);

        fill_output(&shared, &mut [0.0; 160], &mut gain);
        assert_eq!(
            shared.position(),
            PlaybackPosition::from_millis(60),
            "past the join: 160 frames played, 100 of them the old track's"
        );
        assert_eq!(
            shared.duration_ms.load(Ordering::Relaxed),
            2_000,
            "and the length shown is the new track's"
        );
        assert_eq!(
            shared.advances.load(Ordering::Relaxed),
            1,
            "the queue is told once"
        );
    }

    #[test]
    fn a_fade_is_announced_in_its_middle_and_still_reads_the_truth() {
        let shared = shared();
        ready(&shared);
        shared.ring.push(&[1.0; 400]);

        // A fade beginning at frame 20 and running 100 frames: the new track's
        // clock starts at 20, and the handover is said at 70.
        shared.mark_boundary(20, 70, DurationMs::from_secs(2));

        let mut gain = 1.0;
        fill_output(&shared, &mut [0.0; 120], &mut gain);
        assert_eq!(
            shared.advances.load(Ordering::Relaxed),
            0,
            "sixty frames in, the track leaving is still the louder one"
        );

        fill_output(&shared, &mut [0.0; 40], &mut gain);
        assert_eq!(
            shared.advances.load(Ordering::Relaxed),
            1,
            "past the middle, the window is told"
        );
        assert_eq!(
            shared.position(),
            PlaybackPosition::from_millis(60),
            "and it reads eighty frames of output minus the twenty before the \
             new track began — half a fade in, which is the truth"
        );
    }

    #[test]
    fn mono_is_heard_from_both_speakers() {
        let mut out = Vec::new();
        map_channels(&[0.5, -0.5], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_into_one_speaker_is_averaged_rather_than_dropped() {
        let mut out = Vec::new();
        map_channels(&[1.0, 0.0], 2, 1, &mut out);
        assert_eq!(out, vec![0.5]);
    }

    #[test]
    fn a_matching_layout_is_copied_untouched() {
        let mut out = Vec::new();
        map_channels(&[0.1, 0.2, 0.3, 0.4], 2, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }
}
