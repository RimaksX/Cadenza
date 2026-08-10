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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};

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
    frames_played: AtomicU64,
    /// Length of the loaded track in milliseconds.
    pub(crate) duration_ms: AtomicU64,
    /// Set by the decoder when no more samples are coming.
    pub(crate) ended: AtomicBool,
    /// Whether a track is loaded at all.
    pub(crate) loaded: AtomicBool,
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
            duration_ms: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            loaded: AtomicBool::new(false),
            flush_seq: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            flush_base: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            failure: Mutex::new(None),
        }
    }

    /// Sets the output level.
    pub(crate) fn set_volume(&self, volume: Volume) {
        self.volume
            .store(amplitude(volume).to_bits(), Ordering::Relaxed);
    }

    /// Where playback has reached.
    pub(crate) fn position(&self) -> PlaybackPosition {
        let frames = self.frames_played.load(Ordering::Relaxed);
        PlaybackPosition::from_millis(frames * 1_000 / u64::from(self.rate))
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
pub(crate) fn fill_output(shared: &Shared, out: &mut [f32], gain: &mut f32) {
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

    let frames = taken / usize::from(shared.channels);
    shared
        .frames_played
        .fetch_add(frames as u64, Ordering::Relaxed);
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

/// The decode thread's own state.
struct Producer {
    shared: Arc<Shared>,
    source: Option<TrackStream>,
    resampler: Option<Resampling>,
    /// Frames converted but not yet accepted by the ring.
    carry: Vec<f32>,
    /// How much of `carry` has been handed over.
    carry_offset: usize,
    /// Channel-mapped frames, kept between passes to avoid reallocating.
    mapped: Vec<f32>,
}

impl Producer {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            source: None,
            resampler: None,
            carry: Vec::new(),
            carry_offset: 0,
            mapped: Vec::new(),
        }
    }

    /// True when there is nothing left to decode or hand over.
    ///
    /// A finished track counts as idle even though its file is still open: it is
    /// held so that the listener can seek back into it.
    fn is_idle(&self) -> bool {
        let exhausted = self.source.is_none() || self.shared.ended.load(Ordering::Relaxed);
        exhausted && self.carry_offset >= self.carry.len()
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Load { path, reply } => {
                let outcome = self.load(&path);
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
        let source = TrackStream::open(path)?;
        let info = source.info();

        let resampler = if info.sample_rate == self.shared.rate {
            None
        } else {
            Some(Resampling::new(
                info.sample_rate,
                self.shared.rate,
                self.shared.channels,
            )?)
        };

        self.source = Some(source);
        self.resampler = resampler;
        self.carry.clear();
        self.carry_offset = 0;

        self.shared.set_failure(None);
        self.shared
            .duration_ms
            .store(info.duration.as_millis(), Ordering::Relaxed);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.loaded.store(true, Ordering::Relaxed);
        self.shared.flush(PlaybackPosition::START);

        Ok(info)
    }

    fn seek(&mut self, position: PlaybackPosition) -> Result<PlaybackPosition> {
        let Some(source) = self.source.as_mut() else {
            return Err(CoreError::Audio("nothing is loaded to seek in".into()));
        };

        let landed = source.seek(position)?;

        self.carry.clear();
        self.carry_offset = 0;
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }

        // The track may have run out earlier and been seeked back into.
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.flush(landed);

        Ok(landed)
    }

    fn unload(&mut self) {
        self.source = None;
        self.resampler = None;
        self.carry.clear();
        self.carry_offset = 0;

        self.shared.loaded.store(false, Ordering::Relaxed);
        self.shared.ended.store(true, Ordering::Relaxed);
        self.shared.duration_ms.store(0, Ordering::Relaxed);
        self.shared.flush(PlaybackPosition::START);
    }

    /// Moves one step of work along. Returns false when there was nothing to do.
    fn pump(&mut self) -> bool {
        if self.carry_offset < self.carry.len() {
            let accepted = self.shared.ring.push(&self.carry[self.carry_offset..]);
            self.carry_offset += accepted;
            return accepted > 0;
        }

        // Nothing more to read, or the file is finished and only being kept for
        // a seek.
        if self.source.is_none() || self.shared.ended.load(Ordering::Relaxed) {
            return false;
        }

        // Do not decode further ahead than the ring can hold.
        if self.shared.ring.available() == self.shared.ring.capacity() {
            return false;
        }

        let (source_channels, decoded) = {
            let source = self.source.as_mut().expect("checked above");
            let channels = source.info().channels;
            match source.next_frames() {
                Ok(Some(frames)) => (channels, Ok(frames)),
                Ok(None) => (channels, Err(None)),
                Err(err) => (channels, Err(Some(err.to_string()))),
            }
        };

        let decoded = match decoded {
            Ok(frames) => frames,
            Err(failure) => {
                self.finish(failure);
                return false;
            }
        };

        map_channels(
            decoded,
            source_channels,
            self.shared.channels,
            &mut self.mapped,
        );

        self.carry.clear();
        self.carry_offset = 0;
        match self.resampler.as_mut() {
            Some(resampler) => match resampler.process(&self.mapped) {
                Ok(resampled) => self.carry.extend_from_slice(resampled),
                Err(err) => {
                    self.finish(Some(err.to_string()));
                    return false;
                }
            },
            None => self.carry.extend_from_slice(&self.mapped),
        }

        self.carry_offset = self.shared.ring.push(&self.carry);
        true
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

    use cadenza_core::domain::value_objects::{PlaybackPosition, Volume};
    use cadenza_testkit::TempDir;
    use cadenza_testkit::audio_fixtures::write_wav;

    use super::{Producer, Shared, amplitude, fill_output, map_channels};

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
