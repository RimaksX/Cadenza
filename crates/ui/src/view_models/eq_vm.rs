//! Turning the equaliser into something that can be drawn.
//!
//! Two coordinate systems meet here. The listener's is hertz and decibels; the
//! screen's is a fraction across and a fraction down. Every conversion between
//! them lives in this file, so a control that reads wrong reads wrong in one
//! place.

use cadenza_core::domain::eq::{EqPreset, EqSetting};
use cadenza_core::domain::policies::eq_policy::{MAX_BAND_HZ, MIN_BAND_HZ};
use cadenza_core::domain::value_objects::gain::{MAX_GAIN_DB, MIN_GAIN_DB};

use crate::{EqBandData, EqPresetData};

/// How many points the drawn curve is made of.
///
/// Enough that the eye reads a curve rather than a polygon, few enough that
/// rebuilding it on every drag costs nothing.
const CURVE_POINTS: usize = 96;

/// The height of the coordinate space the curve is drawn in.
///
/// Slint scales a path from its viewbox to the space it is given, so the
/// numbers here are arbitrary as long as they are the same in both directions.
const VIEWBOX: f32 = 1_000.0;

/// Where a frequency sits across the screen, from 0 to 1.
///
/// Logarithmic, because that is how frequency is heard: the octave from 100 to
/// 200 Hz takes as much room as the one from 5 to 10 kHz, and a linear axis
/// would spend three quarters of the screen on the top two octaves.
pub fn frequency_to_x(frequency_hz: f32) -> f32 {
    let low = (MIN_BAND_HZ as f32).log10();
    let high = (MAX_BAND_HZ as f32).log10();
    ((frequency_hz.max(1.0).log10() - low) / (high - low)).clamp(0.0, 1.0)
}

/// The frequency at a fraction across the screen.
pub fn x_to_frequency(x: f32) -> u32 {
    let low = (MIN_BAND_HZ as f32).log10();
    let high = (MAX_BAND_HZ as f32).log10();
    let hz = 10.0_f32.powf(low + x.clamp(0.0, 1.0) * (high - low));
    (hz.round() as u32).clamp(MIN_BAND_HZ, MAX_BAND_HZ)
}

/// Where a gain sits down the screen, from 0 at the top.
pub fn gain_to_y(gain_db: f32) -> f32 {
    ((MAX_GAIN_DB - gain_db) / (MAX_GAIN_DB - MIN_GAIN_DB)).clamp(0.0, 1.0)
}

/// The gain at a fraction down the screen.
pub fn y_to_gain(y: f32) -> f32 {
    MAX_GAIN_DB - y.clamp(0.0, 1.0) * (MAX_GAIN_DB - MIN_GAIN_DB)
}

/// The bells, as the screen places them.
pub fn bands(setting: &EqSetting, selected: usize) -> Vec<EqBandData> {
    setting
        .advanced
        .iter()
        .enumerate()
        .map(|(index, band)| EqBandData {
            index: index as i32,
            x: frequency_to_x(band.frequency_hz() as f32),
            y: gain_to_y(band.gain().as_db()),
            frequency: hertz(band.frequency_hz()).into(),
            q: format!("Q {:.1}", band.q()).into(),
            gain: decibels(band.gain().as_db()).into(),
            selected: index == selected,
        })
        .collect()
}

/// The response curve, as a path in its own square.
///
/// Each band contributes a bell in decibels, added up. It is a **drawing**
/// rather than a measurement: the real filters are second-order sections whose
/// response is not quite this shape near the ends of the spectrum. What it gets
/// right is the part somebody reads — where the lift is, how wide, how far —
/// and it costs a fraction of what evaluating eight transfer functions at a
/// hundred points would.
pub fn curve(setting: &EqSetting) -> String {
    let mut path = String::with_capacity(CURVE_POINTS * 16);

    for point in 0..CURVE_POINTS {
        let x = point as f32 / (CURVE_POINTS - 1) as f32;
        let frequency = x_to_frequency(x) as f32;

        let db: f32 = setting
            .advanced
            .iter()
            .map(|band| {
                let octaves = (frequency / band.frequency_hz() as f32).log2();
                // A Gaussian in log frequency: `q` narrows it, exactly as it
                // narrows the filter it stands for.
                band.gain().as_db() * (-(octaves * band.q()).powi(2)).exp()
            })
            .sum();

        let position = (
            x * VIEWBOX,
            gain_to_y(db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)) * VIEWBOX,
        );

        if point == 0 {
            path.push_str(&format!("M {:.1} {:.1}", position.0, position.1));
        } else {
            path.push_str(&format!(" L {:.1} {:.1}", position.0, position.1));
        }
    }

    path
}

/// The presets, built-in and the listener's own.
pub fn presets(all: &[EqPreset], setting: &EqSetting) -> Vec<EqPresetData> {
    all.iter()
        .map(|preset| EqPresetData {
            id: preset.id.to_string().into(),
            name: preset.name.to_uppercase().into(),
            // Lit when what is playing is exactly this preset. Nudge one
            // control and nothing is lit, which is the truth: the sound is no
            // longer any preset at all.
            selected: EqSetting::from(preset) == *setting,
            editable: !preset.is_builtin,
        })
        .collect()
}

/// The line under the heading: which mode, and whether anything is being done.
pub fn summary_line(setting: &EqSetting) -> String {
    let mode = match setting.mode {
        cadenza_core::domain::eq::EqMode::Simple => "three controls",
        cadenza_core::domain::eq::EqMode::Advanced => "eight bands",
    };
    if setting.is_flat() {
        format!("{mode} · flat")
    } else {
        format!("{mode} · in use")
    }
}

/// A gain, with its sign, the way a control reads it out.
pub fn decibels(db: f32) -> String {
    if db.abs() < 0.05 {
        "0".to_owned()
    } else {
        format!("{db:+.1}")
    }
}

/// A frequency, short enough to sit under a node.
pub fn hertz(frequency_hz: u32) -> String {
    if frequency_hz >= 1_000 {
        let thousands = frequency_hz as f32 / 1_000.0;
        if (thousands - thousands.round()).abs() < 0.05 {
            format!("{:.0}k", thousands)
        } else {
            format!("{thousands:.1}k")
        }
    } else {
        format!("{frequency_hz}")
    }
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::eq::{EqBand, EqSetting};
    use cadenza_core::domain::value_objects::GainDb;

    use super::{curve, decibels, frequency_to_x, gain_to_y, hertz, x_to_frequency, y_to_gain};

    #[test]
    fn the_frequency_axis_gives_every_octave_the_same_room() {
        let low = frequency_to_x(100.0) - frequency_to_x(50.0);
        let high = frequency_to_x(10_000.0) - frequency_to_x(5_000.0);
        assert!(
            (low - high).abs() < 0.01,
            "an octave at the bottom took {low} and one at the top took {high}"
        );
    }

    #[test]
    fn a_position_and_a_frequency_are_the_same_thing_said_twice() {
        for frequency in [20, 100, 440, 1_000, 8_000, 20_000] {
            let there_and_back = x_to_frequency(frequency_to_x(frequency as f32));
            let drift = (there_and_back as f32 - frequency as f32).abs() / frequency as f32;
            assert!(drift < 0.01, "{frequency} Hz came back as {there_and_back}");
        }
    }

    #[test]
    fn the_top_of_the_screen_is_the_loudest_a_band_goes() {
        assert_eq!(gain_to_y(12.0), 0.0);
        assert_eq!(gain_to_y(-12.0), 1.0);
        assert!((gain_to_y(0.0) - 0.5).abs() < f32::EPSILON);
        assert!((y_to_gain(0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn a_flat_setting_draws_a_flat_line() {
        let drawn = curve(&EqSetting::flat());
        assert!(drawn.starts_with("M 0.0 500.0"));
        assert!(
            drawn.contains(" L 1000.0 500.0"),
            "and it stays in the middle all the way across: {drawn}"
        );
    }

    #[test]
    fn a_lifted_band_bends_the_curve_where_it_sits() {
        let mut setting = EqSetting::flat();
        setting.advanced[3] =
            EqBand::new(1_000, 1.0, GainDb::new(12.0).expect("in range")).expect("in range");

        let drawn = curve(&setting);
        let points: Vec<f32> = drawn
            .split(['M', 'L'])
            .filter_map(|pair| pair.split_whitespace().nth(1))
            .filter_map(|value| value.parse().ok())
            .collect();

        let highest = points.iter().fold(f32::MAX, |lowest, y| lowest.min(*y));
        assert!(highest < 100.0, "the peak reached {highest} of a thousand");
        assert!(
            points.first().is_some_and(|y| (*y - 500.0).abs() < 20.0),
            "and the bottom of the spectrum is untouched"
        );
    }

    #[test]
    fn a_gain_reads_out_with_its_sign_and_a_frequency_short() {
        assert_eq!(decibels(0.0), "0");
        assert_eq!(decibels(4.0), "+4.0");
        assert_eq!(decibels(-7.5), "-7.5");

        assert_eq!(hertz(60), "60");
        assert_eq!(hertz(1_000), "1k");
        assert_eq!(hertz(2_500), "2.5k");
        assert_eq!(hertz(16_000), "16k");
    }
}
