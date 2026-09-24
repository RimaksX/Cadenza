//! Equaliser gain in decibels.

use crate::{CoreError, Result};

/// Maximum cut an equaliser band may apply.
///
/// A calibration knob. Wider ranges look impressive and clip in practice; ±12 dB
/// is the range hardware graphic equalisers use. Widen it only with headroom
/// handling in the mixer to match.
pub const MIN_GAIN_DB: f32 = -12.0;

/// Maximum boost an equaliser band may apply.
pub const MAX_GAIN_DB: f32 = 12.0;

/// A gain adjustment in decibels, constrained to [`MIN_GAIN_DB`]..=[`MAX_GAIN_DB`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct GainDb(f32);

impl GainDb {
    /// No adjustment.
    pub const ZERO: Self = Self(0.0);

    /// Validates and wraps a decibel value.
    pub fn new(db: f32) -> Result<Self> {
        if !db.is_finite() {
            return Err(CoreError::invalid("gain", "must be a finite number"));
        }
        if !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&db) {
            return Err(CoreError::invalid(
                "gain",
                format!("{db} dB is outside {MIN_GAIN_DB}..={MAX_GAIN_DB} dB"),
            ));
        }
        Ok(Self(db))
    }

    /// Clamps a decibel value into range, for values arriving from a UI slider.
    pub fn clamped(db: f32) -> Self {
        if db.is_nan() {
            return Self::ZERO;
        }
        Self(db.clamp(MIN_GAIN_DB, MAX_GAIN_DB))
    }

    /// The gain in decibels.
    pub const fn as_db(self) -> f32 {
        self.0
    }

    /// The gain as a linear amplitude multiplier.
    pub fn as_amplitude(self) -> f32 {
        10.0_f32.powf(self.0 / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{GainDb, MAX_GAIN_DB, MIN_GAIN_DB};

    #[test]
    fn range_is_enforced_and_clamping_is_opt_in() {
        assert!(GainDb::new(MIN_GAIN_DB).is_ok());
        assert!(GainDb::new(MAX_GAIN_DB).is_ok());
        assert!(GainDb::new(-20.0).is_err());
        assert!(GainDb::new(f32::NAN).is_err());
        assert_eq!(GainDb::clamped(-20.0).as_db(), MIN_GAIN_DB);
        assert_eq!(GainDb::clamped(f32::NAN), GainDb::ZERO);
    }

    #[test]
    fn decibels_convert_to_amplitude() {
        assert!((GainDb::ZERO.as_amplitude() - 1.0).abs() < 1e-6);

        let six_db = GainDb::new(6.0).expect("in range");
        assert!(
            (six_db.as_amplitude() - 2.0).abs() < 0.01,
            "+6 dB roughly doubles amplitude, got {}",
            six_db.as_amplitude()
        );

        let minus_six_db = GainDb::new(-6.0).expect("in range");
        assert!((minus_six_db.as_amplitude() - 0.5).abs() < 0.01);
    }
}
