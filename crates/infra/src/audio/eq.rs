//! The equaliser chain, as the audio callback owns it.
//!
//! It lives on the realtime thread on purpose. PROJECT_MASTER 8.2 lists DSP
//! among the things the callback may do, and it is the only place where moving
//! a control is heard at once: applied on the decode side, a change would reach
//! the speakers as much as two seconds later, which is the prebuffer.
//!
//! What crosses from the control side is not coefficients but the three numbers
//! a band is made of — where it sits, how wide it is, how far it lifts —
//! published atomically. The chain walks its own copy towards them and rebuilds
//! its coefficients once per block. That is what makes a control silent to
//! drag: a step in the coefficients is a step in the waveform, and a step in
//! the waveform is a click (PROJECT_MASTER 2.8, 8.5).

use std::sync::atomic::Ordering;

use cadenza_core::domain::eq::EqMode;
use cadenza_core::domain::policies::eq_policy::{
    ADVANCED_BAND_COUNT, GAIN_RAMP, MAX_BAND_HZ, MAX_BAND_Q, MIN_BAND_HZ, MIN_BAND_Q,
};
use cadenza_core::domain::value_objects::gain::{MAX_GAIN_DB, MIN_GAIN_DB};

use super::biquad::{Coefficients, Section};
use super::stream::Shared;

/// The most bands any mode uses: the eight of the parametric equaliser.
pub(crate) const MAX_BANDS: usize = ADVANCED_BAND_COUNT;

/// How far a frequency may travel in one ramp, in octaves.
///
/// Ten covers the whole audible range, so dragging a band from one end of the
/// spectrum to the other takes exactly as long as any other control's full
/// travel — and a small nudge is over almost at once.
const OCTAVES_PER_RAMP: f32 = 10.0;

/// The three shapes of PROJECT_MASTER 8.5.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Everything below the corner, together.
    LowShelf,
    /// A bell of the given width.
    Peaking,
    /// Everything above the corner, together.
    HighShelf,
}

/// What a band is at any instant: where, how wide, how loud.
#[derive(Debug, Clone, Copy)]
struct Setting {
    frequency_hz: f32,
    q: f32,
    gain_db: f32,
}

impl Setting {
    /// A band that does nothing, parked in the middle of the spectrum.
    const NEUTRAL: Self = Self {
        frequency_hz: 1_000.0,
        q: 1.0,
        gain_db: 0.0,
    };

    fn coefficients(self, rate: u32, shape: Shape) -> Coefficients {
        match shape {
            Shape::LowShelf => Coefficients::low_shelf(rate, self.frequency_hz, self.gain_db),
            Shape::Peaking => Coefficients::peaking(rate, self.frequency_hz, self.gain_db, self.q),
            Shape::HighShelf => Coefficients::high_shelf(rate, self.frequency_hz, self.gain_db),
        }
    }
}

/// What shape the band at `index` takes.
///
/// The simple mode's outer two are shelves: a bell at 100 Hz would leave the
/// octave below it untouched, which is heard as a bass control that does
/// nothing to the lowest notes. Every parametric band is a bell, because that
/// is what parametric means — the listener places it themselves, and a shelf
/// they cannot move off the end of the spectrum would be one control short.
fn shape_of(mode: EqMode, index: usize) -> Shape {
    match (mode, index) {
        (EqMode::Simple, 0) => Shape::LowShelf,
        (EqMode::Simple, 2) => Shape::HighShelf,
        _ => Shape::Peaking,
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
    mode: EqMode,
    active: usize,
    /// Where each band is now.
    current: [Setting; MAX_BANDS],
    /// Where it is going.
    target: [Setting; MAX_BANDS],
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

        Self {
            rate,
            channels,
            state: vec![Section::default(); MAX_BANDS * channels],
            mode: EqMode::Simple,
            active: 0,
            current: [Setting::NEUTRAL; MAX_BANDS],
            target: [Setting::NEUTRAL; MAX_BANDS],
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
        let active = shared.eq_band_count().min(MAX_BANDS);

        // A change of mode is a change of what the bands *are*, so what the
        // filters are holding belongs to a chain that no longer exists.
        if active != self.active
            || !matches!(
                (mode, self.mode),
                (EqMode::Simple, EqMode::Simple) | (EqMode::Advanced, EqMode::Advanced)
            )
        {
            for section in &mut self.state {
                section.reset();
            }
            self.current = self.target;
        }

        self.mode = mode;
        self.active = active;
        self.seen = published;
        self.idle = false;

        for band in 0..MAX_BANDS {
            self.target[band] = if band < active {
                let (frequency_hz, q, gain_db) = shared.eq_band(band);
                Setting {
                    frequency_hz: frequency_hz.clamp(MIN_BAND_HZ as f32, MAX_BAND_HZ as f32),
                    q: if q.is_finite() {
                        q.clamp(MIN_BAND_Q, MAX_BAND_Q)
                    } else {
                        1.0
                    },
                    gain_db: gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB),
                }
            } else {
                Setting::NEUTRAL
            };
        }
    }

    /// Walks every band one block closer to where it is going, and rebuilds the
    /// coefficients of the ones that moved.
    ///
    /// All three numbers travel, not only the gain: a parametric band is
    /// dragged across the spectrum as well as up and down, and a frequency that
    /// jumped would step the waveform exactly as a gain that jumped does. The
    /// frequency moves in octaves rather than hertz, because that is how it is
    /// heard and how the screen draws it.
    fn advance(&mut self, frames: usize) {
        let seconds = GAIN_RAMP.as_millis() as f32 / 1_000.0;
        let fraction = frames as f32 / (seconds * self.rate as f32);

        let gain_step = (MAX_GAIN_DB - MIN_GAIN_DB) * fraction;
        let q_step = (MAX_BAND_Q - MIN_BAND_Q) * fraction;
        let octave_step = OCTAVES_PER_RAMP * fraction;

        for band in 0..self.active {
            let (current, target) = (self.current[band], self.target[band]);

            let gain_db =
                current.gain_db + (target.gain_db - current.gain_db).clamp(-gain_step, gain_step);
            let q = current.q + (target.q - current.q).clamp(-q_step, q_step);

            let octaves = (target.frequency_hz / current.frequency_hz).log2();
            let frequency_hz =
                current.frequency_hz * 2.0_f32.powf(octaves.clamp(-octave_step, octave_step));

            if gain_db == current.gain_db && q == current.q && frequency_hz == current.frequency_hz
            {
                continue;
            }

            self.current[band] = Setting {
                frequency_hz,
                q,
                gain_db,
            };
            self.coefficients[band] =
                self.current[band].coefficients(self.rate, shape_of(self.mode, band));
        }
    }

    /// True when nothing is boosted, cut, or on its way to being either.
    fn settled(&self) -> bool {
        (0..self.active)
            .all(|band| self.current[band].gain_db == 0.0 && self.target[band].gain_db == 0.0)
    }
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::eq::EqMode;
    use cadenza_core::domain::policies::eq_policy::{
        SIMPLE_BASS_HZ, SIMPLE_MID_HZ, SIMPLE_TREBLE_HZ,
    };

    use super::EqChain;
    use crate::audio::stream::Shared;

    const RATE: u32 = 44_100;

    /// A chain and the state it reads from, at one channel so the buffers in
    /// these tests are frames.
    fn chain() -> (Shared, EqChain) {
        (Shared::new(RATE, 1), EqChain::new(RATE, 1))
    }

    /// The three tone controls, as the engine flattens them.
    fn tone_controls(bass: f32, mid: f32, treble: f32) -> Vec<(u32, f32, f32)> {
        vec![
            (SIMPLE_BASS_HZ, 0.7, bass),
            (SIMPLE_MID_HZ, 0.7, mid),
            (SIMPLE_TREBLE_HZ, 0.7, treble),
        ]
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
        shared.set_eq(EqMode::Simple, &tone_controls(8.0, 0.0, 0.0));

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
    fn a_parametric_band_moves_the_frequency_it_was_placed_at() {
        let (shared, mut eq) = chain();
        // One bell at 1 kHz, the rest flat where they were put.
        let mut bands = vec![(60, 1.0, 0.0); 8];
        bands[3] = (1_000, 1.4, 9.0);
        shared.set_eq(EqMode::Advanced, &bands);

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
    fn a_band_answers_where_it_is_put_rather_than_where_it_started() {
        let (shared, mut eq) = chain();
        let mut bands = vec![(60, 1.0, 0.0); 8];

        // The same band, moved from 200 Hz to 4 kHz: a parametric equaliser is
        // one whose bands are dragged, and the filter has to follow.
        bands[0] = (200, 1.4, 9.0);
        shared.set_eq(EqMode::Advanced, &bands);
        assert!(tone_through(&shared, &mut eq, 200.0) > 7.0);

        bands[0] = (4_000, 1.4, 9.0);
        shared.set_eq(EqMode::Advanced, &bands);

        let at_the_old_place = tone_through(&shared, &mut eq, 200.0);
        assert!(
            at_the_old_place.abs() < 1.5,
            "200 Hz is still lifted by {at_the_old_place} dB after the band left it"
        );

        let mut eq = EqChain::new(RATE, 1);
        shared.set_eq(EqMode::Advanced, &bands);
        let at_the_new_place = tone_through(&shared, &mut eq, 4_000.0);
        assert!(
            (at_the_new_place - 9.0).abs() < 1.5,
            "4 kHz came out at {at_the_new_place} dB rather than 9"
        );
    }

    #[test]
    fn a_gain_change_arrives_gradually_rather_than_at_once() {
        let (shared, mut eq) = chain();
        shared.set_eq(EqMode::Simple, &tone_controls(12.0, 0.0, 0.0));

        // One block of a hundred frames, which is a fraction of the fifty
        // millisecond ramp: the gain cannot have arrived yet.
        let mut buffer = vec![1.0_f32; 100];
        eq.process(&shared, &mut buffer);

        assert!(
            eq.current[0].gain_db > 0.0 && eq.current[0].gain_db < 12.0,
            "the gain jumped straight to {} dB",
            eq.current[0].gain_db
        );
    }

    #[test]
    fn a_dragged_band_never_steps_the_waveform() {
        let (shared, mut eq) = chain();
        let step = std::f32::consts::TAU * 60.0 / RATE as f32;
        let mut sample = 0_u32;
        let mut heard: Vec<f32> = Vec::new();

        // A hand on the curve: the gain climbing a decibel at a time while the
        // band itself is dragged across two octaves.
        for tick in 0..=12 {
            let mut bands = vec![(1_000, 1.0, 0.0); 8];
            bands[0] = (60 + tick * 20, 1.4, tick as f32);
            shared.set_eq(EqMode::Advanced, &bands);

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
        shared.set_eq(EqMode::Simple, &tone_controls(10.0, 0.0, 0.0));
        tone_through(&shared, &mut eq, 60.0);

        shared.set_eq(EqMode::Simple, &tone_controls(0.0, 0.0, 0.0));
        let back = tone_through(&shared, &mut eq, 60.0);

        assert!(back.abs() < 0.2, "flat left the tone {back} dB off");
        assert!(eq.idle, "and the chain went back to being skipped");
    }
}
