//! Output volume as a normalised scalar.

use crate::{CoreError, Result};

/// Playback volume in `0.0..=1.0`.
///
/// This is the *control* value — what the slider shows and what gets persisted.
/// The perceptual taper that converts it into a linear amplitude belongs to the
/// audio chain and belongs to the mixer.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Volume(f32);

impl Volume {
    /// Silence.
    pub const MUTED: Self = Self(0.0);

    /// Unattenuated output.
    pub const FULL: Self = Self(1.0);

    /// Validates and wraps a scalar, rejecting NaN and out-of-range input.
    pub fn new(value: f32) -> Result<Self> {
        if !value.is_finite() {
            return Err(CoreError::invalid("volume", "must be a finite number"));
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(CoreError::invalid(
                "volume",
                format!("{value} is outside 0.0..=1.0"),
            ));
        }
        Ok(Self(value))
    }

    /// Clamps a scalar into range. For values arriving from a UI slider, where
    /// refusing the input would be worse than snapping it.
    pub fn clamped(value: f32) -> Self {
        if value.is_nan() {
            return Self::MUTED;
        }
        Self(value.clamp(0.0, 1.0))
    }

    /// The scalar in `0.0..=1.0`.
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// True when the volume is exactly zero.
    pub fn is_muted(self) -> bool {
        self.0 == 0.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::FULL
    }
}

#[cfg(test)]
mod tests {
    use super::Volume;

    #[test]
    fn range_is_enforced() {
        assert!(Volume::new(0.0).is_ok());
        assert!(Volume::new(1.0).is_ok());
        assert!(Volume::new(-0.01).is_err());
        assert!(Volume::new(1.01).is_err());
    }

    #[test]
    fn nan_and_infinity_are_rejected() {
        assert!(Volume::new(f32::NAN).is_err());
        assert!(Volume::new(f32::INFINITY).is_err());
    }

    #[test]
    fn clamping_snaps_instead_of_failing() {
        assert_eq!(Volume::clamped(2.0), Volume::FULL);
        assert_eq!(Volume::clamped(-1.0), Volume::MUTED);
        assert_eq!(Volume::clamped(f32::NAN), Volume::MUTED);
    }
}
