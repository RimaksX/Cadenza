//! Tempo in beats per minute.

use crate::{CoreError, Result};

/// Slowest tempo the analyser is allowed to report.
pub const MIN_BPM: f32 = 20.0;

/// Fastest tempo the analyser is allowed to report.
///
/// Beat trackers routinely report double or half time; anything past this is a
/// detection failure, not a fast track, and should be discarded rather than fed
/// into transition scoring.
pub const MAX_BPM: f32 = 300.0;

/// A tempo estimate in beats per minute.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Bpm(f32);

impl Bpm {
    /// Validates and wraps a tempo.
    pub fn new(value: f32) -> Result<Self> {
        if !value.is_finite() {
            return Err(CoreError::invalid("bpm", "must be a finite number"));
        }
        if !(MIN_BPM..=MAX_BPM).contains(&value) {
            return Err(CoreError::invalid(
                "bpm",
                format!("{value} is outside {MIN_BPM}..={MAX_BPM}"),
            ));
        }
        Ok(Self(value))
    }

    /// The tempo in beats per minute.
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Absolute tempo difference against another estimate.
    pub fn delta(self, other: Self) -> f32 {
        (self.0 - other.0).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::{Bpm, MAX_BPM, MIN_BPM};

    #[test]
    fn implausible_tempos_are_rejected() {
        assert!(Bpm::new(MIN_BPM).is_ok());
        assert!(Bpm::new(MAX_BPM).is_ok());
        assert!(Bpm::new(0.0).is_err());
        assert!(Bpm::new(500.0).is_err());
        assert!(Bpm::new(f32::NAN).is_err());
    }

    #[test]
    fn delta_is_symmetric() {
        let slow = Bpm::new(90.0).expect("in range");
        let fast = Bpm::new(128.0).expect("in range");
        assert!((slow.delta(fast) - 38.0).abs() < f32::EPSILON);
        assert!((fast.delta(slow) - 38.0).abs() < f32::EPSILON);
    }
}
