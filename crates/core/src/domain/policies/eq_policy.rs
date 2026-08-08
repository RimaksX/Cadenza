//! Equaliser band layout and validation.

use crate::domain::eq::EqBand;
use crate::domain::value_objects::{DurationMs, GainDb};
use crate::{CoreError, Result};

/// Centre frequencies of the ten-band graphic equaliser (PROJECT_MASTER 8.5).
pub const ADVANCED_BAND_FREQUENCIES: [u32; 10] =
    [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000];

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
/// shorter feels more responsive, longer is safer against zipper noise. The
/// mixer in M9 consumes this.
pub const GAIN_RAMP: DurationMs = DurationMs::from_millis(50);

/// The number of bands an advanced preset must have.
pub const ADVANCED_BAND_COUNT: usize = ADVANCED_BAND_FREQUENCIES.len();

/// A flat ten-band configuration.
pub fn default_advanced_bands() -> Vec<EqBand> {
    ADVANCED_BAND_FREQUENCIES
        .iter()
        .map(|&frequency_hz| EqBand {
            frequency_hz,
            gain: GainDb::ZERO,
        })
        .collect()
}

/// Checks that a stored advanced preset matches the expected band layout.
///
/// Validated on load rather than trusted: a preset file edited by hand, or one
/// written by an older version with a different layout, would otherwise map
/// gains onto the wrong frequencies and quietly wreck the sound.
pub fn validate_advanced_bands(bands: &[EqBand]) -> Result<()> {
    if bands.len() != ADVANCED_BAND_COUNT {
        return Err(CoreError::invalid(
            "eq bands",
            format!("expected {ADVANCED_BAND_COUNT} bands, got {}", bands.len()),
        ));
    }
    for (band, &expected) in bands.iter().zip(ADVANCED_BAND_FREQUENCIES.iter()) {
        if band.frequency_hz != expected {
            return Err(CoreError::invalid(
                "eq bands",
                format!(
                    "expected {expected} Hz at this position, got {}",
                    band.frequency_hz
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ADVANCED_BAND_COUNT, ADVANCED_BAND_FREQUENCIES, default_advanced_bands,
        validate_advanced_bands,
    };
    use crate::domain::eq::EqBand;
    use crate::domain::value_objects::GainDb;

    #[test]
    fn the_bands_are_the_ten_from_the_specification_in_order() {
        assert_eq!(ADVANCED_BAND_COUNT, 10);
        assert!(
            ADVANCED_BAND_FREQUENCIES
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "frequencies must ascend"
        );
        assert_eq!(ADVANCED_BAND_FREQUENCIES[0], 31);
        assert_eq!(ADVANCED_BAND_FREQUENCIES[9], 16_000);
    }

    #[test]
    fn the_default_preset_is_flat_and_valid() {
        let bands = default_advanced_bands();
        assert!(validate_advanced_bands(&bands).is_ok());
        assert!(bands.iter().all(|band| band.gain == GainDb::ZERO));
    }

    #[test]
    fn a_wrong_band_count_is_rejected() {
        let mut bands = default_advanced_bands();
        bands.pop();
        assert!(validate_advanced_bands(&bands).is_err());
    }

    #[test]
    fn a_reordered_or_retuned_layout_is_rejected() {
        let mut bands = default_advanced_bands();
        bands[3] = EqBand {
            frequency_hz: 300,
            gain: GainDb::ZERO,
        };
        assert!(
            validate_advanced_bands(&bands).is_err(),
            "a shifted centre frequency would apply gains to the wrong band"
        );
    }
}
