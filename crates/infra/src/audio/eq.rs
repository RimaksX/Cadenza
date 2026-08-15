//! The equaliser chain, as the audio callback owns it.
//!
//! It lives on the realtime thread on purpose. PROJECT_MASTER 8.2 lists DSP
//! among the things the callback may do, and it is the only place where moving
//! a slider is heard at once: applied on the decode side, a change would reach
//! the speakers as much as two seconds later, which is the prebuffer.
//!
//! What crosses from the control side is not coefficients but **gains in
//! decibels**, published atomically. The chain walks its own gains towards them
//! and rebuilds its coefficients once per block, which is what makes a slider
//! silent to drag: a step in the coefficients is a step in the waveform, and a
//! step in the waveform is a click (PROJECT_MASTER 2.8, 8.5).

use std::sync::atomic::Ordering;

use cadenza_core::domain::eq::EqMode;
use cadenza_core::domain::policies::eq_policy::{
    ADVANCED_BAND_FREQUENCIES, GAIN_RAMP, SIMPLE_BASS_HZ, SIMPLE_MID_HZ, SIMPLE_TREBLE_HZ,
};
use cadenza_core::domain::value_objects::gain::{MAX_GAIN_DB, MIN_GAIN_DB};

use super::biquad::{Coefficients, Section};
use super::stream::Shared;

/// The most bands any mode uses: the ten of the graphic equaliser.
pub(crate) const MAX_BANDS: usize = ADVANCED_BAND_FREQUENCIES.len();

/// How wide each bell of the ten-band layout is.
///
/// The bands are an octave apart, and `sqrt(2)` is the Q whose bell is one
/// octave wide between its half-power points — so the ten of them cover the
/// spectrum once each rather than piling up on their neighbours.
const OCTAVE_Q: f32 = std::f32::consts::SQRT_2;

/// How wide the simple mode's one bell is.
///
/// Much broader than a band of the graphic equaliser, because it is not a band:
/// "mid" is everything between the bass and the treble, and a narrow bell there
/// would be a tone control that only moved one note.
const MID_Q: f32 = 0.7;

/// What one band of the chain is.
#[derive(Debug, Clone, Copy)]
struct Band {
    frequency_hz: f32,
    shape: Shape,
}

/// The three shapes of PROJECT_MASTER 8.5.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Everything below the corner, together.
    LowShelf,
    /// A bell of the given width.
    Peaking(f32),
    /// Everything above the corner, together.
    HighShelf,
}

impl Band {
    fn coefficients(self, rate: u32, gain_db: f32) -> Coefficients {
        match self.shape {
            Shape::LowShelf => Coefficients::low_shelf(rate, self.frequency_hz, gain_db),
            Shape::Peaking(q) => Coefficients::peaking(rate, self.frequency_hz, gain_db, q),
            Shape::HighShelf => Coefficients::high_shelf(rate, self.frequency_hz, gain_db),
        }
    }
}

/// The bands a mode is made of.
///
/// Both layouts put a shelf at each end and bells in between: a bell at the
/// bottom band leaves the octave below it untouched, which is audible as a
/// bass control that does nothing to the lowest notes.
fn layout(mode: EqMode) -> ([Band; MAX_BANDS], usize) {
    let mut bands = [Band {
        frequency_hz: 0.0,
        shape: Shape::Peaking(OCTAVE_Q),
    }; MAX_BANDS];

    match mode {
        EqMode::Simple => {
            bands[0] = Band {
                frequency_hz: SIMPLE_BASS_HZ as f32,
                shape: Shape::LowShelf,
            };
            bands[1] = Band {
                frequency_hz: SIMPLE_MID_HZ as f32,
                shape: Shape::Peaking(MID_Q),
            };
            bands[2] = Band {
                frequency_hz: SIMPLE_TREBLE_HZ as f32,
                shape: Shape::HighShelf,
            };
            (bands, 3)
        }
        EqMode::Advanced => {
            for (band, &frequency_hz) in bands.iter_mut().zip(ADVANCED_BAND_FREQUENCIES.iter()) {
                band.frequency_hz = frequency_hz as f32;
            }
            bands[0].shape = Shape::LowShelf;
            bands[MAX_BANDS - 1].shape = Shape::HighShelf;
            (bands, MAX_BANDS)
        }
    }
}

/// The equaliser, and the filter memory it needs to keep.
///
/// Owned by the callback the way its gain ramp is: state that belongs to the
/// stream of samples rather than to the application, and so is never shared.
#[derive(Debug)]
pub(crate) struct EqChain {
    rate: u32,
    channels: usize,
    /// One section per band for every channel. Allocated once, at the top of
    /// the stream, and never resized.
    state: Vec<Section>,
    bands: [Band; MAX_BANDS],
    active: usize,
    /// Where each band's gain is now, in decibels.
    current: [f32; MAX_BANDS],
    /// Where it is going.
    target: [f32; MAX_BANDS],
    coefficients: [Coefficients; MAX_BANDS],
    /// The published change this chain has taken up.
    seen: u64,
    /// True once every band is flat *and* the filter memory has drained, which
    /// is when the whole chain can be skipped.
    idle: bool,
}

impl EqChain {
    /// Builds a flat chain for one output format.
    pub(crate) fn new(rate: u32, channels: u16) -> Self {
        let channels = usize::from(channels.max(1));
        let (bands, active) = layout(EqMode::Simple);

        Self {
            rate,
            channels,
            state: vec![Section::default(); MAX_BANDS * channels],
            bands,
            active,
            current: [0.0; MAX_BANDS],
            target: [0.0; MAX_BANDS],
            coefficients: [Coefficients::IDENTITY; MAX_BANDS],
            seen: 0,
            idle: true,
        }
    }

    /// Applies the equaliser to one output buffer. **Realtime.**
    ///
    /// Reads atomics, multiplies and adds. No allocation, no locking, no
    /// branches on anything the control side could be holding.
    pub(crate) fn process(&mut self, shared: &Shared, out: &mut [f32]) {
        let published = shared.eq_seq.load(Ordering::Acquire);
        if published != self.seen {
            self.retarget(shared, published);
        }

        if self.idle {
            return;
        }

        let frames = out.len() / self.channels;
        self.advance(frames);

        for frame in out.chunks_mut(self.channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                for band in 0..self.active {
                    let section = &mut self.state[channel * MAX_BANDS + band];
                    *sample = section.step(&self.coefficients[band], *sample);
                }
            }
        }

        // Flat, and this block was the one that drained what the filters were
        // still holding: from here the chain can be skipped entirely, which is
        // what the common case deserves to cost.
        if self.settled() {
            self.idle = true;
            for section in &mut self.state {
                section.reset();
            }
        }
    }

    /// Takes up a change published by the control side.
    fn retarget(&mut self, shared: &Shared, published: u64) {
        let mode = shared.eq_mode();
        let (bands, active) = layout(mode);

        // A change of mode is a change of what the bands *are*, so the gains
        // held for the old layout mean nothing under the new one. They start
        // from where they were rather than from zero, which is the same
        // compromise a ramp always is: the alternative is a dip to flat and
        // back, and that is louder than the change itself.
        if active != self.active {
            for section in &mut self.state {
                section.reset();
            }
        }

        self.bands = bands;
        self.active = active;
        self.seen = published;
        self.idle = false;

        for band in 0..MAX_BANDS {
            self.target[band] = if band < active {
                shared.eq_gain(band).clamp(MIN_GAIN_DB, MAX_GAIN_DB)
            } else {
                0.0
            };
        }
    }

    /// Walks every gain one block closer to where it is going, and rebuilds the
    /// coefficients of the ones that moved.
    fn advance(&mut self, frames: usize) {
        // The whole range in exactly one ramp, so a slider dragged from end to
        // end takes as long to arrive as one nudged by a decibel takes to
        // settle in proportion.
        let seconds = GAIN_RAMP.as_millis() as f32 / 1_000.0;
        let step = (MAX_GAIN_DB - MIN_GAIN_DB) * frames as f32 / (seconds * self.rate as f32);

        for band in 0..self.active {
            let distance = self.target[band] - self.current[band];
            if distance == 0.0 {
                continue;
            }
            self.current[band] += distance.clamp(-step, step);
            self.coefficients[band] = self.bands[band].coefficients(self.rate, self.current[band]);
        }
    }

    /// True when nothing is boosted, cut, or on its way to being either.
    fn settled(&self) -> bool {
        self.current[..self.active].iter().all(|db| *db == 0.0)
            && self.target[..self.active].iter().all(|db| *db == 0.0)
    }
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::eq::EqMode;

    use super::{EqChain, MAX_BANDS};
    use crate::audio::stream::Shared;

    const RATE: u32 = 44_100;

    /// A chain and the state it reads from, at one channel so the buffers in
    /// these tests are frames.
    fn chain() -> (Shared, EqChain) {
        (Shared::new(RATE, 1), EqChain::new(RATE, 1))
    }

    /// Pushes a steady tone through and reports its level in decibels, after
    /// giving the ramp long enough to arrive.
    fn tone_through(shared: &Shared, eq: &mut EqChain, frequency_hz: f32) -> f32 {
        let step = std::f32::consts::TAU * frequency_hz / RATE as f32;
        let mut sample = 0_u32;
        let mut peak = 0.0_f32;

        // Twenty blocks of a thousand frames: half a second, against a fifty
        // millisecond ramp.
        for block in 0..20 {
            let mut buffer: Vec<f32> = (0..1_000)
                .map(|_| {
                    let value = (step * sample as f32).sin();
                    sample += 1;
                    value
                })
                .collect();

            eq.process(shared, &mut buffer);

            // Only the last blocks count: the first are the ramp and the
            // filter's own start-up.
            if block >= 15 {
                peak = peak.max(
                    buffer
                        .iter()
                        .fold(0.0_f32, |largest, value| largest.max(value.abs())),
                );
            }
        }

        20.0 * peak.log10()
    }

    #[test]
    fn a_flat_equaliser_is_skipped_altogether() {
        let (shared, mut eq) = chain();
        let mut buffer = vec![0.5; 64];

        eq.process(&shared, &mut buffer);

        assert!(buffer.iter().all(|sample| *sample == 0.5), "untouched");
    }

    #[test]
    fn the_bass_control_lifts_the_bass_and_leaves_the_treble_alone() {
        let (shared, mut eq) = chain();
        shared.set_eq(EqMode::Simple, &[8.0, 0.0, 0.0]);

        let low = tone_through(&shared, &mut eq, 50.0);
        assert!(
            (low - 8.0).abs() < 1.0,
            "50 Hz came out at {low} dB rather than 8"
        );

        let mut eq = EqChain::new(RATE, 1);
        let high = tone_through(&shared, &mut eq, 10_000.0);
        assert!(high.abs() < 1.0, "10 kHz moved by {high} dB and should not");
    }

    #[test]
    fn a_band_of_the_graphic_equaliser_moves_its_own_octave() {
        let (shared, mut eq) = chain();
        let mut gains = [0.0; MAX_BANDS];
        // 1 kHz is the sixth of the ten.
        gains[5] = 9.0;
        shared.set_eq(EqMode::Advanced, &gains);

        let at_band = tone_through(&shared, &mut eq, 1_000.0);
        assert!(
            (at_band - 9.0).abs() < 1.5,
            "1 kHz came out at {at_band} dB rather than 9"
        );

        let mut eq = EqChain::new(RATE, 1);
        let elsewhere = tone_through(&shared, &mut eq, 125.0);
        assert!(
            elsewhere.abs() < 1.5,
            "125 Hz moved by {elsewhere} dB, three octaves from the band that changed"
        );
    }

    #[test]
    fn a_gain_change_arrives_gradually_rather_than_at_once() {
        let (shared, mut eq) = chain();
        shared.set_eq(EqMode::Simple, &[12.0, 0.0, 0.0]);

        // One block of a hundred frames, which is a fraction of the fifty
        // millisecond ramp: the gain cannot have arrived yet.
        let mut buffer = vec![1.0_f32; 100];
        eq.process(&shared, &mut buffer);

        assert!(
            eq.current[0] > 0.0 && eq.current[0] < 12.0,
            "the gain jumped straight to {} dB",
            eq.current[0]
        );
    }

    #[test]
    fn a_dragged_control_never_steps_the_waveform() {
        let (shared, mut eq) = chain();
        let step = std::f32::consts::TAU * 60.0 / RATE as f32;
        let mut sample = 0_u32;
        let mut heard: Vec<f32> = Vec::new();

        // Twelve decibels of bass, arriving a decibel at a time the way a hand
        // on a slider delivers it.
        for decibels in 0..=12 {
            shared.set_eq(EqMode::Simple, &[decibels as f32, 0.0, 0.0]);

            let mut buffer: Vec<f32> = (0..2_000)
                .map(|_| {
                    let value = (step * sample as f32).sin();
                    sample += 1;
                    value
                })
                .collect();
            eq.process(&shared, &mut buffer);
            heard.extend_from_slice(&buffer);
        }

        // The tone itself steps by this much between neighbouring samples; a
        // coefficient applied in one jump would show up as much more.
        let tone_step = (step.sin() * 4.0).abs();
        let largest = heard
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max);

        assert!(
            largest < tone_step,
            "the waveform jumped by {largest}, and the tone itself only moves {tone_step} a sample"
        );
    }

    #[test]
    fn returning_to_flat_puts_the_signal_back_as_it_was() {
        let (shared, mut eq) = chain();
        shared.set_eq(EqMode::Simple, &[10.0, 0.0, 0.0]);
        tone_through(&shared, &mut eq, 60.0);

        shared.set_eq(EqMode::Simple, &[0.0, 0.0, 0.0]);
        let back = tone_through(&shared, &mut eq, 60.0);

        assert!(back.abs() < 0.2, "flat left the tone {back} dB off");
        assert!(eq.idle, "and the chain went back to being skipped");
    }
}
