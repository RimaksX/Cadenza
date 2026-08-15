//! The audio engine: cpal output, and the port the application calls.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cadenza_core::domain::eq::{EqMode, EqSetting};
use cadenza_core::domain::playback::{PlaybackState, TransitionProfile};
use cadenza_core::domain::policies::eq_policy::{SIMPLE_BASS_HZ, SIMPLE_MID_HZ, SIMPLE_TREBLE_HZ};
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::settings::CrossfadeDuration;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

use super::eq::EqChain;
use super::stream::{Command, Shared, decode_loop, fill_output};

/// How wide the simple mode's mid bell is, and the shelves' nominal width.
///
/// Broad, because "mid" is not a band: it is everything between the bass and
/// the treble, and a narrow bell there would be a tone control that only moved
/// one note. The shelves ignore it and take their slope from the cookbook.
const SIMPLE_MID_Q: f32 = 0.7;

/// How long the caller waits for the output device to open.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a control call waits for the decode thread to answer.
///
/// Generous, because opening a file means reading its header off a disk that may
/// be asleep. It exists so that a wedged decoder surfaces as an error rather
/// than as a frozen interface.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Plays audio through the default output device.
///
/// The cpal stream is not `Send`, and [`AudioEnginePort`] is `Send + Sync`, so
/// the stream lives on a thread of its own that does nothing but hold it open.
/// What crosses between threads is [`Shared`], which is only atomics and a ring.
pub struct CpalAudioEngine {
    shared: Arc<Shared>,
    /// `Sender` is `Send` but not `Sync`; the mutex is what makes the engine
    /// shareable. It is never locked from the audio callback.
    commands: Mutex<Sender<Command>>,
    /// Dropping this is how the output thread is told to let go of the stream.
    keepalive: Mutex<Option<Sender<()>>>,
    output_thread: Mutex<Option<JoinHandle<()>>>,
    decode_thread: Mutex<Option<JoinHandle<()>>>,
    /// Device name, rate and channel count, for diagnostics.
    description: String,
}

impl CpalAudioEngine {
    /// Opens the default output device and starts the engine.
    pub fn new() -> Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (alive_tx, alive_rx) = mpsc::channel();

        let output_thread = thread::Builder::new()
            .name("cadenza-audio-output".into())
            .spawn(move || output_thread(&ready_tx, &alive_rx))
            .map_err(|err| CoreError::Audio(format!("cannot start the audio thread: {err}")))?;

        let (shared, description) = ready_rx
            .recv_timeout(START_TIMEOUT)
            .map_err(|_| CoreError::Audio("the audio device did not open in time".into()))??;

        let (command_tx, command_rx) = mpsc::channel();
        let decode_thread = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("cadenza-audio-decode".into())
                .spawn(move || decode_loop(shared, &command_rx))
                .map_err(|err| CoreError::Audio(format!("cannot start the decode thread: {err}")))?
        };

        Ok(Self {
            shared,
            commands: Mutex::new(command_tx),
            keepalive: Mutex::new(Some(alive_tx)),
            output_thread: Mutex::new(Some(output_thread)),
            decode_thread: Mutex::new(Some(decode_thread)),
            description,
        })
    }

    /// The device the engine opened, and the format it is running at.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Length of the loaded track.
    pub fn duration(&self) -> DurationMs {
        self.shared.duration()
    }

    /// How many times the device asked for samples that were not ready.
    ///
    /// Zero on a healthy machine. Anything else is the decoder failing to keep
    /// up, and is the first number worth looking at when playback stutters.
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// Why playback stopped, when it stopped for a reason worth reporting.
    pub fn failure(&self) -> Option<String> {
        self.shared
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The crossfade length last set.
    pub fn crossfade(&self) -> DurationMs {
        DurationMs::from_millis(self.shared.crossfade_ms.load(Ordering::Relaxed))
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(command)
            .map_err(|_| CoreError::Audio("the audio engine has stopped".into()))
    }

    /// Sends a command and waits for the decode thread's answer.
    fn request<T>(&self, command: Command, replies: &Receiver<Result<T>>) -> Result<T> {
        self.send(command)?;
        replies
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|_| CoreError::Audio("the decoder did not answer".into()))?
    }
}

impl AudioEnginePort for CpalAudioEngine {
    fn load(&self, path: &Path) -> Result<()> {
        let (reply, replies) = mpsc::channel();
        self.request(
            Command::Load {
                path: path.to_path_buf(),
                reply,
            },
            &replies,
        )?;
        Ok(())
    }

    fn preload_next(&self, path: &Path, transition: TransitionProfile) -> Result<()> {
        let (reply, replies) = mpsc::channel();
        self.request(
            Command::Preload {
                path: path.to_path_buf(),
                transition,
                reply,
            },
            &replies,
        )
    }

    fn play(&self) -> Result<()> {
        if !self.shared.loaded.load(Ordering::Relaxed) {
            return Err(CoreError::Audio("nothing is loaded to play".into()));
        }
        self.shared.playing.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn pause(&self) -> Result<()> {
        self.shared.playing.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&self) -> Result<()> {
        self.shared.playing.store(false, Ordering::Relaxed);
        self.send(Command::Stop)
    }

    fn seek(&self, position: PlaybackPosition) -> Result<()> {
        let (reply, replies) = mpsc::channel();
        self.request(Command::Seek { position, reply }, &replies)?;
        Ok(())
    }

    fn set_volume(&self, volume: Volume) -> Result<()> {
        self.shared.set_volume(volume);
        Ok(())
    }

    fn armed(&self) -> bool {
        self.shared.armed.load(Ordering::Relaxed)
    }

    fn advances(&self) -> u64 {
        self.shared.advances.load(Ordering::Relaxed)
    }

    fn set_crossfade(&self, duration: CrossfadeDuration) -> Result<()> {
        self.shared
            .crossfade_ms
            .store(duration.as_duration().as_millis(), Ordering::Relaxed);
        Ok(())
    }

    fn set_eq(&self, setting: &EqSetting) -> Result<()> {
        // Both modes are flattened to the same three numbers a filter needs.
        // Which of them is a shelf is the chain's business, and it works that
        // out from the mode and the band's place in it.
        let bands: Vec<(u32, f32, f32)> = match setting.mode {
            EqMode::Simple => vec![
                (SIMPLE_BASS_HZ, SIMPLE_MID_Q, setting.simple.bass.as_db()),
                (SIMPLE_MID_HZ, SIMPLE_MID_Q, setting.simple.mid.as_db()),
                (
                    SIMPLE_TREBLE_HZ,
                    SIMPLE_MID_Q,
                    setting.simple.treble.as_db(),
                ),
            ],
            EqMode::Advanced => setting
                .advanced
                .iter()
                .map(|band| (band.frequency_hz(), band.q(), band.gain().as_db()))
                .collect(),
        };

        // A small allocation on a control call, which is the side of the ring
        // where allocating is allowed. What crosses to the callback is the
        // fixed array of atomics inside `Shared`.
        self.shared.set_eq(setting.mode, &bands);
        Ok(())
    }

    fn position(&self) -> PlaybackPosition {
        self.shared.position()
    }

    fn state(&self) -> PlaybackState {
        if !self.shared.loaded.load(Ordering::Relaxed) {
            return PlaybackState::Stopped;
        }
        // The decoder finished and the device has caught up: the track is over.
        // Until the ring empties there is still audio owed to the listener.
        if self.shared.ended.load(Ordering::Relaxed) && self.shared.ring.is_empty() {
            return PlaybackState::Stopped;
        }
        if self.shared.playing.load(Ordering::Relaxed) {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }
}

impl Drop for CpalAudioEngine {
    fn drop(&mut self) {
        // Stop decoding first: the decode thread holds a file open, and would
        // otherwise keep filling a ring nobody is emptying.
        let _ = self.send(Command::Shutdown);
        if let Some(handle) = self
            .decode_thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = handle.join();
        }

        // Dropping the sender ends the output thread's wait, which drops the
        // stream and closes the device.
        drop(
            self.keepalive
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        if let Some(handle) = self
            .output_thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = handle.join();
        }
    }
}

/// Holds the cpal stream open for the life of the engine.
///
/// A thread of its own, because `cpal::Stream` is not `Send`: it cannot be
/// stored in a `Sync` engine, and dropping it is what closes the device.
fn output_thread(ready: &Sender<Result<(Arc<Shared>, String)>>, alive: &Receiver<()>) {
    match start_output() {
        Ok((shared, stream, description)) => {
            if ready.send(Ok((shared, description))).is_err() {
                return;
            }
            // Returns as soon as the engine drops its end.
            let _ = alive.recv();
            drop(stream);
        }
        Err(err) => {
            let _ = ready.send(Err(err));
        }
    }
}

/// Opens the default device and starts a stream on it.
fn start_output() -> Result<(Arc<Shared>, cpal::Stream, String)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| CoreError::Audio("no audio output device is available".into()))?;

    let name = device
        .description()
        .map_or_else(|_| "unnamed device".to_owned(), |it| it.name().to_owned());
    let supported = device
        .default_output_config()
        .map_err(|err| CoreError::Audio(format!("{name} has no usable output format: {err}")))?;

    if supported.sample_format() != SampleFormat::F32 {
        // ponytail: f32 only. Shared-mode WASAPI mixes in f32, and Windows is
        // the whole target platform (PROJECT_MASTER 1.2). Supporting integer
        // formats means one more conversion in the callback and hardware to
        // test it on.
        return Err(CoreError::Audio(format!(
            "{name} wants {:?} samples, and Cadenza produces 32-bit float",
            supported.sample_format()
        )));
    }

    let config: StreamConfig = supported.config();
    let description = format!(
        "{name} — {} Hz, {} channels",
        config.sample_rate, config.channels
    );

    let shared = Arc::new(Shared::new(config.sample_rate, config.channels));

    let callback_shared = Arc::clone(&shared);
    let error_shared = Arc::clone(&shared);
    // The callback's own gain, so a ramp continues across buffer boundaries.
    let mut gain = 0.0_f32;
    // And its own filters. Their memory belongs to the stream of samples, not
    // to the application, so it is never shared and never locked.
    let mut eq = EqChain::new(shared.rate, shared.channels);

    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [f32], _| fill_output(&callback_shared, out, &mut gain, &mut eq),
            move |err| {
                // Called when the device itself fails. Nothing here can fix it;
                // recording why lets the interface say something truthful.
                error_shared.set_failure(Some(format!("audio device error: {err}")));
            },
            None,
        )
        .map_err(|err| CoreError::Audio(format!("{name} refused the stream: {err}")))?;

    stream
        .play()
        .map_err(|err| CoreError::Audio(format!("{name} did not start: {err}")))?;

    Ok((shared, stream, description))
}
