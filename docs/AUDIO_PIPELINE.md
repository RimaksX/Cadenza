# Audio pipeline

Normative definition: `PROJECT_MASTER.json`, section `8_Audio_pipeline`.

Status: playing since M5, joining tracks since M8, equalised since M9, and
tapped for the visualiser since M10.

## Signal chain

Specified:

`TrackDecoder -> PCM frames -> Resampler -> EQ chain -> stream volume / fade -> Mixer
-> visualizer tap -> audio output`

Built so far:

`TrackStream -> channel map -> Resampling -> mixer -> SampleRing -> gain -> cpal output`

Two `TrackStream`s at once through the transition, which is what the mixer is
for. It sits **before** the ring, on the decode thread, rather than on the
callback — see `MASTER_ISSUES` 38 for why, and what it costs.

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
| decode | opens files, decodes, resamples, mixes the join, fills the ring | yes |
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

## Transitions

A track does not end for the next one to begin. The decoder is told what follows
while the current track is still playing, opens it, and produces the join
itself; the callback plays out one unbroken stream of samples and never learns
that anything happened.

- **Gapless** is concatenation. When the outgoing file runs out, the next lane's
  samples are the next samples pushed. Nothing is timed, so nothing can be
  mistimed — this is what "sample-accurate" means here.
- **Crossfade** overlaps them by the stored length, using equal-power gains
  (`cos`/`sin`, so `cos² + sin² = 1`). A linear fade dips 3 dB in the middle and
  is heard as a hole. A block is cut short so the fade begins on the exact frame
  rather than at whatever buffer boundary falls inside it.
- **A fade takes at most half the outgoing track.** Four seconds is a gentle
  overlap at the end of a song and most of an interlude; on a six-second track
  the stored length would have put two thirds of it underneath its neighbour,
  which is a mix rather than a transition. The length is worked out from the
  track's whole duration, not from what is left of it, so that the same number
  decides when the fade starts and how long it then runs.
- **Which one** is `playback_policy::transition_for`, keyed on the origin of the
  track that is *playing*: radio and playlists are continuous material and stay
  gapless whatever the switch says (PROJECT_MASTER 2.4). A container that will
  not state its length cannot be faded on a schedule, so it joins gapless.

### Telling the application

The callback owns the clock, so it also owns the moment of the handover:

- The decoder publishes **where** the new track begins and **when to say so**,
  as frame indices on the same ruler both ends count on, before a single sample
  of it is queued.
- Those are not the same frame. A gapless join is one instant and both are it,
  but a crossfade is seconds long, and through its first half the louder track
  is still the one leaving. Naming the incoming track over audio that is mostly
  the outgoing one reads as a bug; the handover is said at the **middle** of the
  fade, where what is heard changes over.
- The clock is still rebased to where the new track actually began, so the
  position shown at that moment is a truthful two seconds in rather than zero.
- The callback crosses the mark, rebases the position onto the new track, and
  bumps a counter.
- `QueueService::poll` compares that counter with what it last saw and moves the
  queue's own bookkeeping on — **without touching the engine**. Loading the
  track that is already playing would flush the ring and put a hole in the
  middle of the join.

Nothing here is a callback into the application: section 8.2 forbids it, so the
application asks, four times a second, at the cost of two atomic loads.

### Arming

`QueueService::poll` also arms the next track, by asking `armed()` rather than
by remembering that it did. The engine drops what it had armed whenever the
ground moves — a hard load, a seek — and only the engine knows when that was.

Preloading is a per-profile setting and is on by default. Crossfade is off by
default and, until there is a settings screen, is turned on from the command
line:

```text
cadenza crossfade on 4
cadenza crossfade off
```

### What is not covered by tests

Gapless MP3 is "as far as the format allows" (PROJECT_MASTER 2.4): whether the
encoder's delay and padding are trimmed is Symphonia's business, not Cadenza's.
And "no clicks" is judged by ear. What the tests can assert is the objective
half — that the waveform does not step at the join — and they do.

## The visualiser

The tap is the **last** stage, after the equaliser and after the gain: what is
drawn is what is heard. The callback's whole part in it is a copy into a ring —
section 8.2 allows passing frames to a tap and nothing else — and the copy is
skipped entirely while nobody is looking.

Everything else runs on the thread that asks:

- **Thirty times a second**, which is the ceiling section 2.9 sets, on a timer
  of its own. The transport's tick is four a second and stays there.
- **A window of 1024 samples**, summed to mono and shaped by a Hann window.
  Without the window the ends of the block are a step, and a step is broadband:
  every bar would carry a little of every other.
- **Eight bars, spaced by octave.** A linear spacing gives the top octave half
  the bars and the bottom two none; the ear hears ratios.
- **Decibels, floored at −40.** This number is the whole difference between a
  row of bars that dances and one that looks painted on. Music at a normal
  level lives between about −40 and −15 dB: over a sixty decibel range that is
  a third of the height and a wobble of two pixels, which is what the first
  version drew and what the owner reported as "they do not move".
- **Fast up, slow down.** A bar that falls as fast as it rises spends its time
  at the extremes; the asymmetry is what makes it look like sound.

Nothing is read while nothing is playing, and the tap is turned off with it —
so the cost when the music stops is zero rather than small.

### What it costs

Measured on a Ryzen 7 3700X, release build, as a share of **one** core:

| | |
|---|---|
| Decoding, equaliser and output, visualiser off | 0.3% |
| With the visualiser, as first written | 15.1% |
| With it, after the trimming below | 5.7% |

The surprise is where the cost is not. The transform is a tenth of a per cent;
the engine is a third of one. Everything else is the **window repainting** —
touching the model is what asks for a frame, and a frame is the expensive
thing. Three changes, in order of what they saved:

- **No animation on the bars.** An animated property makes Slint redraw at the
  display's rate whether or not anything new has arrived. The readings are
  twenty a second and that is the animation. (15.1% → 8.1%)
- **Twenty readings a second rather than thirty.** The cap in 2.9 is thirty;
  nothing requires it, and a row of bars still moves like sound at twenty.
- **Only write a bar that moved a whole pixel.** The square gives a bar
  forty-six pixels, so anything finer is a difference the window would round
  away — and every write is a repaint. (→ 5.7%)

A reading's own cost can be measured on its own:

```text
cargo test -p cadenza-infra --lib what_a_reading_costs -- --ignored --nocapture
```

Any number here without the build it came from is worth nothing: the same
binary in a debug build costs 112% of one core, which is seven per cent of a
sixteen-thread machine and is what a listener will report if they run one.
