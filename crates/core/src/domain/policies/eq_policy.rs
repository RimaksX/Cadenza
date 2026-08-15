//! Equaliser band layout and validation.

use crate::domain::eq::EqBand;
use crate::domain::value_objects::{DurationMs, GainDb};
use crate::{CoreError, Result};

/// How many bands the parametric equaliser has (`MASTER_ISSUES` 41).
///
/// Eight rather than the ten fixed bands section 8.5 first described: a
/// parametric band chooses its own frequency, so ten of them are two more
/// things to place rather than two more octaves of control.
pub const ADVANCED_BAND_COUNT: usize = 8;

/// Where a fresh parametric equaliser puts its bands, and how wide.
///
/// Spread roughly by octaves across what a listener can hear, so somebody who
/// opens the advanced mode and starts dragging finds a band near whatever they
/// want to change. A Q of one is a bell about an octave and a half wide: broad
/// enough to be musical, narrow enough to be aimed.
pub const DEFAULT_PARAMETRIC_BANDS: [(u32, f32); ADVANCED_BAND_COUNT] = [
    (60, 1.0),
    (150, 1.0),
    (400, 1.0),
    (1_000, 1.0),
    (2_500, 1.0),
    (6_000, 1.0),
    (10_000, 1.0),
    (16_000, 1.0),
];

/// Lowest frequency a band may be placed at.
///
/// Below twenty hertz is felt rather than heard, and a filter down there costs
/// headroom for something nobody can check.
pub const MIN_BAND_HZ: u32 = 20;

/// Highest frequency a band may be placed at.
pub const MAX_BAND_HZ: u32 = 20_000;

/// Widest a bell may be, as a Q.
///
/// A quarter is broader than the whole midrange: below this a "band" is a tone
/// control that moves everything.
pub const MIN_BAND_Q: f32 = 0.25;

/// Narrowest a bell may be.
///
/// Eight is about a sixth of an octave — narrow enough to notch out a
/// resonance, and not so narrow that it rings audibly on its own.
pub const MAX_BAND_Q: f32 = 8.0;

/// Low shelf corner for the simple equaliser's bass control.
pub const SIMPLE_BASS_HZ: u32 = 100;

/// Peaking centre for the simple equaliser's mid control.
pub const SIMPLE_MID_HZ: u32 = 1_000;

/// High shelf corner for the simple equaliser's treble control.
pub const SIMPLE_TREBLE_HZ: u32 = 8_000;

/// How long a gain change is ramped over instead of being applied instantly.
///
/// PROJECT_MASTER 2.8 and 8.5 require gain changes without clicks; a step change
/// in filter coefficients is exactly what produces one. A calibration knob:
/// shorter feels more responsive, longer is safer against zipper noise.
pub const GAIN_RAMP: DurationMs = DurationMs::from_millis(50);

/// A flat parametric configuration at the default band placement.
pub fn default_advanced_bands() -> Vec<EqBand> {
    DEFAULT_PARAMETRIC_BANDS
        .iter()
        .map(|&(frequency_hz, q)| {
            EqBand::new(frequency_hz, q, GainDb::ZERO).expect("the defaults are in range")
        })
        .collect()
}

/// Checks that a stored parametric preset can be built into filters.
///
/// Validated on load rather than trusted. These numbers reach the realtime
/// filter, where a Q of zero is a division by zero in the coefficients and a
/// frequency past Nyquist is a filter with nothing to work on — and a preset
/// can arrive from a file somebody edited by hand or an older version that
/// stored a different shape.
///
/// What is deliberately *not* checked is the order of the bands. A parametric
/// equaliser is a set of bells rather than a row of sliders; the screen sorts
/// them to draw its curve, and refusing to save an out-of-order set would be a
/// rule with nothing behind it.
pub fn validate_advanced_bands(bands: &[EqBand]) -> Result<()> {
    if bands.len() != ADVANCED_BAND_COUNT {
        return Err(CoreError::invalid(
            "eq bands",
            format!("expected {ADVANCED_BAND_COUNT} bands, got {}", bands.len()),
        ));
    }
    // Each band validates itself on construction; this re-checks so that a
    // collection assembled field by field cannot slip past.
    for band in bands {
        EqBand::new(band.frequency_hz(), band.q(), band.gain())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ADVANCED_BAND_COUNT, DEFAULT_PARAMETRIC_BANDS, MAX_BAND_HZ, MAX_BAND_Q, MIN_BAND_HZ,
        MIN_BAND_Q, default_advanced_bands, validate_advanced_bands,
    };
    use crate::domain::eq::EqBand;
    use crate::domain::value_objects::GainDb;

    #[test]
    fn the_default_placement_spans_what_can_be_heard() {
        assert_eq!(DEFAULT_PARAMETRIC_BANDS.len(), ADVANCED_BAND_COUNT);
        assert!(
            DEFAULT_PARAMETRIC_BANDS
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "the defaults are laid out low to high"
        );
        assert!(DEFAULT_PARAMETRIC_BANDS[0].0 >= MIN_BAND_HZ);
        assert!(
            DEFAULT_PARAMETRIC_BANDS[ADVANCED_BAND_COUNT - 1].0 <= MAX_BAND_HZ,
            "and inside what a band may be placed at"
        );
    }

    #[test]
    fn the_default_preset_is_flat_and_valid() {
        let bands = default_advanced_bands();
        assert!(validate_advanced_bands(&bands).is_ok());
        assert!(bands.iter().all(|band| band.gain() == GainDb::ZERO));
    }

    #[test]
    fn a_wrong_band_count_is_rejected() {
        let mut bands = default_advanced_bands();
        bands.pop();
        assert!(validate_advanced_bands(&bands).is_err());
    }

    #[test]
    fn a_band_outside_what_a_filter_can_be_built_from_is_rejected() {
        assert!(EqBand::new(MIN_BAND_HZ - 1, 1.0, GainDb::ZERO).is_err());
        assert!(EqBand::new(MAX_BAND_HZ + 1, 1.0, GainDb::ZERO).is_err());
        assert!(
            EqBand::new(1_000, 0.0, GainDb::ZERO).is_err(),
            "a Q of zero is a division by zero in the coefficients"
        );
        assert!(EqBand::new(1_000, MAX_BAND_Q + 1.0, GainDb::ZERO).is_err());
        assert!(EqBand::new(1_000, f32::NAN, GainDb::ZERO).is_err());

        assert!(EqBand::new(MIN_BAND_HZ, MIN_BAND_Q, GainDb::ZERO).is_ok());
        assert!(EqBand::new(MAX_BAND_HZ, MAX_BAND_Q, GainDb::ZERO).is_ok());
    }

    #[test]
    fn bands_out_of_order_are_a_shape_rather_than_a_mistake() {
        let mut bands = default_advanced_bands();
        bands.reverse();
        assert!(
            validate_advanced_bands(&bands).is_ok(),
            "a set of bells has no order to be wrong about"
        );
    }
}
