# Audio pipeline

Normative definition: `PROJECT_MASTER.json`, section `8_Audio_pipeline`.

Status: playing since M5. Transitions land in M8, the equaliser in M9, the
visualiser tap in M10.

## Signal chain

Specified:

`TrackDecoder -> PCM frames -> Resampler -> EQ chain -> stream volume / fade -> Mixer
-> visualizer tap -> audio output`

Built so far:

`TrackStream -> channel map -> Resampling -> SampleRing -> gain -> cpal output`

Decoding is Symphonia, output is cpal, resampling is rubato, FFT will be rustfft.
FFmpeg is not used. The channel map is not in section 8.1 and is not optional: a
file's layout and the device's rarely match, and mono played into one channel is
a broken speaker as far as the listener can tell.

The stages still missing insert into this chain rather than replacing it. The
ring is where the chain is cut in two, and that is the whole design: everything
upstream of it may allocate and block, and everything downstream may not.

## Threads

| Thread | Does | May block |
|---|---|---|
| control | `AudioEnginePort` calls: load, play, pause, seek, volume | yes |
| decode | opens files, decodes, resamples, fills the ring | yes |
| callback | empties the ring, applies gain, writes to the device | **no** |

The cpal stream lives on a fourth thread that only holds it open: `cpal::Stream`
is not `Send`, and `AudioEnginePort` is `Send + Sync`.

## Audio thread contract

The realtime callback must never allocate, perform IO, block on a mutex, call
into the UI, or write to the database. It may read from lock-free ring buffers,
apply DSP, mix, read atomically published parameters, and push frames to the
visualiser tap.

The whole callback is one function, `audio/stream.rs::fill_output`. Everything it
is allowed to do is visible there, which is the point of keeping it that small.
The ring stores samples as bit patterns in relaxed atomics rather than behind an
`UnsafeCell`, so the realtime path contains no `unsafe` at all.

Two rules follow from the split, and both are easy to get wrong:

- **The read index belongs to the callback.** A seek cannot empty the ring from
  the decode thread. It raises a flag, the callback drains and acknowledges, and
  only then does the decoder write samples from the new position. Skipping the
  acknowledgement loses fresh samples and drifts the reported position.
- **Position is counted where audio leaves.** `frames_played` advances in the
  callback, not in the decoder, which runs seconds ahead of it.

## Starting, pausing and stopping

- Playback does not start until 50 ms is queued. Without that gate the first
  callback of every track arrives before the decoder has produced anything, and
  every track begins with an underrun.
- Gain ramps over 5 ms rather than jumping, on volume changes and on pause. A
  step in the waveform is what a click is.
- Pause leaves the queue alone. The callback stops consuming once the ramp has
  reached silence.
- A finished track keeps its file open until it is stopped or replaced. The
  decoder reaches the end of the file seconds before the last note is heard, and
  closing it there would make the end of every track unseekable.
- Underruns are counted. `cadenza play` prints the total; anything but zero means
  the decoder is not keeping up.

## Volume

The control value is linear in slider position, and hearing is not. The engine
squares it, putting mid-travel near −12 dB. The tapered value is what the
callback multiplies by.

## Formats

MP3, AAC, ALAC, FLAC and WAV, all decoded by Symphonia and all verified playing
at M5. AAC support is AAC-LC; HE-AAC is not decoded, and a container does not
say which profile it holds until it is opened.

Output is 32-bit float only. Shared-mode WASAPI mixes in f32, so on the target
platform nothing else comes up; a device that wants integer samples is refused
with a message that says so rather than played wrongly.

## Transitions

Crossfade uses equal-power curves, 3–5 s configurable, default 4 s, for ordinary
tracks. Radio and playlists default to gapless with sample-accurate joins and the
next track decoded ahead of time. Neither may introduce clicks or artificial
gaps. None of this exists yet: `preload_next` returns an error rather than
pretending, so a caller cannot mistake a gap for a transition.
