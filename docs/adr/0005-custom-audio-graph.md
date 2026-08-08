# 5. Custom audio graph over cpal

Status: accepted (M0)

## Context

Cadenza needs equal-power crossfade, sample-accurate gapless transitions, a 10-band
equalizer applied without clicks, and a visualizer tap — all without the audio callback
allocating, locking or performing IO. Off-the-shelf playback crates hide the mixer and
the transition point, which is exactly where these requirements live.

## Decision

Build the graph in `crates/infra/src/audio`:

`decoder -> PCM frames -> resampler -> EQ chain -> stream volume/fade -> mixer ->
visualizer tap -> cpal output`

cpal supplies only the output stream. Symphonia decodes, rubato resamples, rustfft
handles analysis and visualization. FFmpeg is not used; an alternative decoder adapter
stays architecturally possible via `DecoderPort` but is not built unless a real
compatibility problem appears.

## Consequences

- Two decoded streams can be alive at once, which is what makes crossfade and gapless
  prebuffering possible at all.
- All parameter changes (volume, EQ coefficients, fade position) reach the audio thread
  through atomics or lock-free ring buffers; nothing on the realtime path allocates or
  blocks.
- EQ coefficient changes are ramped rather than applied instantly, to avoid clicks.
- Cost: mixing, resampling and buffer management are ours to get right and ours to test.
  `docs/TESTING.md` requires DSP-level tests for this reason.
