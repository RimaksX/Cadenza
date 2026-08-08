# Audio pipeline

Normative definition: `PROJECT_MASTER.json`, section `8_Audio_pipeline`.

Status: not implemented. The engine lands in M5, transitions in M8, EQ in M9,
visualization tap in M10.

## Signal chain

`TrackDecoder -> PCM frames -> Resampler -> EQ chain -> stream volume / fade -> Mixer
-> visualizer tap -> audio output`

Decoding is Symphonia, output is cpal, resampling is rubato, FFT is rustfft. FFmpeg is
not used.

## Audio thread contract

The realtime callback must never allocate, perform IO, block on a mutex, call into the
UI, or write to the database. It may read from lock-free ring buffers, apply DSP, mix,
read atomically published parameters, and push frames to the visualizer tap.

Everything else — decoding, prebuffering, parameter computation — happens off the audio
thread and is handed over through lock-free structures.

## Transitions

Crossfade uses equal-power curves, 3–5 s configurable, default 4 s, for ordinary
tracks. Radio and playlists default to gapless with sample-accurate joins and the next
track decoded ahead of time. Neither may introduce clicks or artificial gaps.
