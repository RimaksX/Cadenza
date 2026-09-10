//! One second-order section, and the three shapes the equaliser is built from.
//!
//! The coefficient formulas are Robert Bristow-Johnson's audio cookbook, which
//! is what every graphic equaliser in the world uses: low shelf, peaking and
//! high shelf are the three shapes the equaliser needs, and they are enough for
//! both modes.
//!
//! Nothing here allocates or branches on anything but its arguments, because a
//! [`Section`] is stepped on the realtime callback (which allows
//! DSP there, and nothing else).

/// The coefficients of one normalised second-order section.
///
/// Already divided through by `a0`, so stepping the filter is multiplication
/// and addition and nothing else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// How steep a shelf is.
///
/// One is the steepest a shelf goes without overshooting into a resonant peak
/// either side of its corner — which is a ringing sound, not a tone control. A
/// calibration knob: lower is gentler.
const SHELF_SLOPE: f32 = 1.0;

/// Above this fraction of Nyquist a band is left out.
///
/// A filter centred at or past half the sample rate has nothing to work on: the
/// cookbook's `sin(w0)` goes to zero there and the coefficients degenerate. It
/// matters for the 16 kHz band, which is real at 44.1 kHz and imaginary at 32.
const NYQUIST_MARGIN: f32 = 0.95;

impl Coefficients {
    /// Passes the signal through untouched.
    pub(crate) const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// A bell centred on `frequency_hz`.
    ///
    /// `q` is how wide the bell is: about 1.41 for one octave, which is the
    /// spacing of the ten-band layout, and lower for the broad mid control of
    /// the simple one.
    pub(crate) fn peaking(rate: u32, frequency_hz: f32, gain_db: f32, q: f32) -> Self {
        let Some((_, cos_w0, sin_w0)) = angle(rate, frequency_hz) else {
            return Self::IDENTITY;
        };

        let amplitude = amplitude(gain_db);
        let alpha = sin_w0 / (2.0 * q);

        Self::normalise(
            1.0 + alpha * amplitude,
            -2.0 * cos_w0,
            1.0 - alpha * amplitude,
            1.0 + alpha / amplitude,
            -2.0 * cos_w0,
            1.0 - alpha / amplitude,
        )
    }

    /// Everything below `frequency_hz`, lifted or cut together.
    pub(crate) fn low_shelf(rate: u32, frequency_hz: f32, gain_db: f32) -> Self {
        let Some((_, cos_w0, sin_w0)) = angle(rate, frequency_hz) else {
            return Self::IDENTITY;
        };

        let a = amplitude(gain_db);
        let alpha = shelf_alpha(sin_w0, a);
        let two_root_a_alpha = 2.0 * a.sqrt() * alpha;

        Self::normalise(
            a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_root_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
            a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_root_a_alpha),
            (a + 1.0) + (a - 1.0) * cos_w0 + two_root_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - two_root_a_alpha,
        )
    }

    /// Everything above `frequency_hz`, lifted or cut together.
    pub(crate) fn high_shelf(rate: u32, frequency_hz: f32, gain_db: f32) -> Self {
        let Some((_, cos_w0, sin_w0)) = angle(rate, frequency_hz) else {
            return Self::IDENTITY;
        };

        let a = amplitude(gain_db);
        let alpha = shelf_alpha(sin_w0, a);
        let two_root_a_alpha = 2.0 * a.sqrt() * alpha;

        Self::normalise(
            a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_root_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
            a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_root_a_alpha),
            (a + 1.0) - (a - 1.0) * cos_w0 + two_root_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - two_root_a_alpha,
        )
    }

    /// Divides through by `a0`, which is what makes the section one multiply
    /// per coefficient instead of a division per sample.
    fn normalise(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        if a0.abs() < f32::EPSILON {
            return Self::IDENTITY;
        }
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// The angular frequency of a band, or `None` where it has no room to exist.
fn angle(rate: u32, frequency_hz: f32) -> Option<(f32, f32, f32)> {
    let nyquist = rate as f32 / 2.0;
    if rate == 0 || frequency_hz <= 0.0 || frequency_hz >= nyquist * NYQUIST_MARGIN {
        return None;
    }
    let w0 = std::f32::consts::TAU * frequency_hz / rate as f32;
    Some((w0, w0.cos(), w0.sin()))
}

/// The cookbook's `A`: half a decibel gain, because a shelf or bell applies it
/// twice over — once in the numerator and once in the denominator.
fn amplitude(gain_db: f32) -> f32 {
    10.0_f32.powf(gain_db / 40.0)
}

fn shelf_alpha(sin_w0: f32, a: f32) -> f32 {
    sin_w0 / 2.0
        * ((a + 1.0 / a) * (1.0 / SHELF_SLOPE - 1.0) + 2.0)
            .max(0.0)
            .sqrt()
}

/// One filter's memory, for one channel.
///
/// Transposed direct form II: two words of state instead of four, and better
/// behaved in `f32` than the direct form when the coefficients move underneath
/// it — which is exactly what a listener dragging a slider does.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Section {
    s1: f32,
    s2: f32,
}

impl Section {
    /// Steps the filter by one sample.
    pub(crate) fn step(&mut self, coefficients: &Coefficients, input: f32) -> f32 {
        let output = coefficients.b0 * input + self.s1;
        self.s1 = coefficients.b1 * input - coefficients.a1 * output + self.s2;
        self.s2 = coefficients.b2 * input - coefficients.a2 * output;
        output
    }

    /// Forgets what it has heard.
    pub(crate) fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{Coefficients, Section};

    const RATE: u32 = 44_100;

    /// The amplitude a steady tone comes out at, after the filter has settled.
    ///
    /// A sine at `frequency_hz` is pushed through for long enough that the
    /// start-up transient is behind it, and the largest excursion of what is
    /// left is the gain at that frequency.
    fn response(coefficients: &Coefficients, frequency_hz: f32) -> f32 {
        let mut section = Section::default();
        let step = std::f32::consts::TAU * frequency_hz / RATE as f32;

        let settle = 4_000;
        let measure = 4_000;
        for sample in 0..settle {
            section.step(coefficients, (step * sample as f32).sin());
        }

        let mut peak = 0.0_f32;
        for sample in settle..settle + measure {
            let output = section.step(coefficients, (step * sample as f32).sin());
            peak = peak.max(output.abs());
        }
        peak
    }

    /// Decibels, for comparing against the gain that was asked for.
    fn decibels(amplitude: f32) -> f32 {
        20.0 * amplitude.log10()
    }

    #[test]
    fn a_flat_band_is_the_signal_it_was_given() {
        for coefficients in [
            Coefficients::peaking(RATE, 1_000.0, 0.0, 1.41),
            Coefficients::low_shelf(RATE, 100.0, 0.0),
            Coefficients::high_shelf(RATE, 8_000.0, 0.0),
        ] {
            assert!(
                (decibels(response(&coefficients, 1_000.0))).abs() < 0.05,
                "a gain of zero changed the signal"
            );
        }
    }

    #[test]
    fn a_bell_lifts_its_own_frequency_and_leaves_the_far_ones_alone() {
        let boost = Coefficients::peaking(RATE, 1_000.0, 6.0, 1.41);

        let at_centre = decibels(response(&boost, 1_000.0));
        assert!(
            (at_centre - 6.0).abs() < 0.3,
            "asked for 6 dB at the centre and got {at_centre}"
        );

        // Four octaves down and four up: outside the bell either way.
        for far in [62.0, 16_000.0] {
            let elsewhere = decibels(response(&boost, far));
            assert!(
                elsewhere.abs() < 1.0,
                "{far} Hz moved by {elsewhere} dB, and it is not this band's business"
            );
        }
    }

    #[test]
    fn a_cut_is_the_boost_upside_down() {
        let cut = Coefficients::peaking(RATE, 1_000.0, -6.0, 1.41);
        let at_centre = decibels(response(&cut, 1_000.0));
        assert!(
            (at_centre + 6.0).abs() < 0.3,
            "asked for -6 dB and got {at_centre}"
        );
    }

    #[test]
    fn a_low_shelf_lifts_everything_under_its_corner() {
        let shelf = Coefficients::low_shelf(RATE, 200.0, 8.0);

        let deep = decibels(response(&shelf, 40.0));
        assert!(
            (deep - 8.0).abs() < 0.6,
            "well below the corner the shelf should be at its full 8 dB, not {deep}"
        );

        let above = decibels(response(&shelf, 8_000.0));
        assert!(above.abs() < 0.5, "and well above it, nothing: {above}");
    }

    #[test]
    fn a_high_shelf_lifts_everything_over_its_corner() {
        let shelf = Coefficients::high_shelf(RATE, 4_000.0, 8.0);

        let high = decibels(response(&shelf, 16_000.0));
        assert!(
            (high - 8.0).abs() < 0.6,
            "well above the corner: {high} rather than 8 dB"
        );

        let low = decibels(response(&shelf, 100.0));
        assert!(low.abs() < 0.5, "and well below it, nothing: {low}");
    }

    #[test]
    fn a_band_with_no_room_left_at_the_top_passes_the_signal_through() {
        // 16 kHz asked of a 32 kHz stream is past Nyquist's usable edge. The
        // band is dropped rather than allowed to produce nonsense.
        assert_eq!(
            Coefficients::peaking(32_000, 16_000.0, 12.0, 1.41),
            Coefficients::IDENTITY
        );
        assert_eq!(
            Coefficients::high_shelf(8_000, 16_000.0, 12.0),
            Coefficients::IDENTITY
        );
    }

    #[test]
    fn a_filter_settles_rather_than_running_away() {
        // Every band at full boost, driven hard: an unstable section shows up
        // as an output that keeps growing.
        let coefficients = Coefficients::peaking(RATE, 31.0, 12.0, 1.41);
        let mut section = Section::default();

        let mut last = 0.0_f32;
        for sample in 0..200_000 {
            let step = std::f32::consts::TAU * 31.0 / RATE as f32;
            last = section.step(&coefficients, (step * sample as f32).sin());
        }
        assert!(last.abs() < 8.0, "the filter ran away to {last}");
    }

    #[test]
    fn resetting_forgets_the_signal() {
        let coefficients = Coefficients::low_shelf(RATE, 100.0, 12.0);
        let mut section = Section::default();
        for _ in 0..100 {
            section.step(&coefficients, 1.0);
        }

        section.reset();
        let mut fresh = Section::default();
        assert_eq!(
            section.step(&coefficients, 1.0),
            fresh.step(&coefficients, 1.0)
        );
    }
}
