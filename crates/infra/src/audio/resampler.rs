//! Matching a file's sample rate to the output device's.

use cadenza_core::{CoreError, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// Frames of input consumed per pass.
///
/// Small enough that a seek discards little work, large enough that the filter's
/// per-call overhead disappears against the multiply-accumulates it does.
const CHUNK_FRAMES: usize = 1_024;

/// Length of the interpolation filter.
///
/// 128 taps at 128× oversampling sits in the middle of rubato's own recommended
/// range: audibly transparent for 44.1 kHz to 48 kHz, and cheap enough that
/// resampling stays a rounding error against decoding on the same thread.
const SINC_LEN: usize = 128;

/// Converts interleaved frames from one sample rate to another.
///
/// Windows runs its mixer at one fixed rate — usually 48 kHz — while most music
/// is 44.1 kHz. Playing one at the other's rate shifts the pitch by more than a
/// semitone, so this is not optional (PROJECT_MASTER 3.4).
///
/// Lives on the decode thread. The audio callback never sees it: the filter
/// allocates on construction and its cost per pass varies, and neither is
/// allowed on the realtime side (PROJECT_MASTER 8.2).
pub struct Resampling {
    inner: Async<f32>,
    channels: usize,
    /// Input frames that arrived but did not fill a chunk.
    pending: Vec<f32>,
    /// One pass of output.
    scratch: Vec<f32>,
    /// Everything produced by the current call.
    output: Vec<f32>,
}

impl Resampling {
    /// Builds a resampler from `from` Hz to `to` Hz.
    pub fn new(from: u32, to: u32, channels: u16) -> Result<Self> {
        if from == 0 || to == 0 || channels == 0 {
            return Err(CoreError::Audio(format!(
                "cannot resample {from} Hz to {to} Hz across {channels} channels"
            )));
        }

        let channels = usize::from(channels);
        let parameters =
            SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2)
                .interpolation(SincInterpolationType::Cubic);

        let inner = Async::new_sinc(
            f64::from(to) / f64::from(from),
            // The ratio is fixed for the life of a track — nothing adjusts it
            // while playing — so the allowed range around it can be tight.
            1.1,
            &parameters,
            CHUNK_FRAMES,
            channels,
            FixedAsync::Input,
        )
        .map_err(|err| CoreError::Audio(format!("resampler: {err}")))?;

        let capacity = inner.output_frames_max() * channels;

        Ok(Self {
            inner,
            channels,
            pending: Vec::with_capacity(CHUNK_FRAMES * channels * 2),
            scratch: vec![0.0; capacity],
            output: Vec::with_capacity(capacity),
        })
    }

    /// Feeds interleaved frames in and returns whatever came out.
    ///
    /// The result is often empty — input is buffered until a whole chunk is
    /// available — and is only valid until the next call.
    pub fn process(&mut self, input: &[f32]) -> Result<&[f32]> {
        let Self {
            inner,
            channels,
            pending,
            scratch,
            output,
        } = self;

        output.clear();
        pending.extend_from_slice(input);

        loop {
            let wanted = inner.input_frames_next();
            if pending.len() < wanted * *channels {
                break;
            }

            let source = InterleavedSlice::new(&pending[..wanted * *channels], *channels, wanted)
                .map_err(|err| CoreError::Audio(format!("resampler input: {err}")))?;

            let frames_out = inner.output_frames_next();
            let mut sink = InterleavedSlice::new_mut(
                &mut scratch[..frames_out * *channels],
                *channels,
                frames_out,
            )
            .map_err(|err| CoreError::Audio(format!("resampler output: {err}")))?;

            let (consumed, produced) = inner
                .process_into_buffer(&source, &mut sink, None)
                .map_err(|err| CoreError::Audio(format!("resampling failed: {err}")))?;

            output.extend_from_slice(&scratch[..produced * *channels]);
            pending.drain(..consumed * *channels);
        }

        Ok(&self.output)
    }

    /// Throws away buffered input and the filter's history.
    ///
    /// Called on a seek: the samples either side of a jump are not continuous,
    /// and interpolating across the join would produce a click.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.output.clear();
        self.inner.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::Resampling;

    /// How much material each length test feeds through.
    ///
    /// Long enough that the tail — up to one chunk of input is always still
    /// buffered when the test stops — stays well inside the tolerance below,
    /// while a wrong ratio would miss by several percent.
    const TEST_SECONDS: u32 = 4;

    /// Feeds several seconds in and counts the frames that come out.
    fn resample(from: u32, to: u32, channels: u16) -> usize {
        let mut resampler = Resampling::new(from, to, channels).expect("constructed");
        let frame = vec![0.25_f32; usize::from(channels)];

        let mut produced = 0;
        for _ in 0..from * TEST_SECONDS {
            produced += resampler.process(&frame).expect("resampled").len();
        }
        produced / usize::from(channels)
    }

    #[test]
    fn upsampling_produces_the_output_rate() {
        let frames = resample(44_100, 48_000, 2);
        let expected = f64::from(48_000 * TEST_SECONDS);

        let error = (frames as f64 - expected).abs() / expected;
        assert!(
            error < 0.01,
            "44.1 kHz to 48 kHz produced {frames} frames, expected about {expected}"
        );
    }

    #[test]
    fn downsampling_loses_frames_in_the_same_proportion() {
        let frames = resample(48_000, 44_100, 2);
        let expected = f64::from(44_100 * TEST_SECONDS);

        let error = (frames as f64 - expected).abs() / expected;
        assert!(
            error < 0.01,
            "48 kHz to 44.1 kHz produced {frames} frames, expected about {expected}"
        );
    }

    #[test]
    fn silence_stays_silent() {
        let mut resampler = Resampling::new(44_100, 48_000, 2).expect("constructed");
        let silence = vec![0.0_f32; 4_096];

        for _ in 0..4 {
            for sample in resampler.process(&silence).expect("resampled") {
                assert!(
                    sample.abs() < 1e-6,
                    "the filter invented {sample} out of silence"
                );
            }
        }
    }

    #[test]
    fn less_than_a_chunk_produces_nothing_and_loses_nothing() {
        let mut resampler = Resampling::new(44_100, 48_000, 2).expect("constructed");
        assert!(resampler.process(&[0.1; 64]).expect("resampled").is_empty());
    }

    #[test]
    fn a_nonsense_rate_is_refused_rather_than_dividing_by_zero() {
        assert!(Resampling::new(0, 48_000, 2).is_err());
        assert!(Resampling::new(44_100, 0, 2).is_err());
        assert!(Resampling::new(44_100, 48_000, 0).is_err());
    }
}
