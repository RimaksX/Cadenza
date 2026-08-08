//! A span of time in milliseconds.

use std::fmt;

/// A non-negative span of time, in milliseconds.
///
/// This is the unit the database stores (`duration_ms`, `played_ms`) and the unit
/// the audio pipeline reasons in, so no conversion happens at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DurationMs(u64);

impl DurationMs {
    /// Zero length.
    pub const ZERO: Self = Self(0);

    /// Wraps a millisecond count.
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Wraps a second count.
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(1_000))
    }

    /// The span in milliseconds.
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// The span truncated to whole seconds.
    pub const fn as_secs(self) -> u64 {
        self.0 / 1_000
    }

    /// The span in fractional seconds.
    pub fn as_secs_f32(self) -> f32 {
        self.0 as f32 / 1_000.0
    }

    /// True when the span is zero.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Adds two spans, saturating at [`u64::MAX`].
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Subtracts `other`, saturating at zero.
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// How much of `total` this span covers, clamped to `0.0..=1.0`.
    ///
    /// A zero `total` yields `0.0` rather than a division by zero, which is what a
    /// progress bar wants for a track whose duration is not known yet.
    pub fn fraction_of(self, total: Self) -> f32 {
        if total.is_zero() {
            return 0.0;
        }
        (self.0 as f32 / total.0 as f32).clamp(0.0, 1.0)
    }
}

impl fmt::Display for DurationMs {
    /// Renders as `m:ss`, or `h:mm:ss` once the span reaches an hour.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.as_secs();
        let (hours, minutes, seconds) = (total / 3_600, (total % 3_600) / 60, total % 60);
        if hours > 0 {
            write!(f, "{hours}:{minutes:02}:{seconds:02}")
        } else {
            write!(f, "{minutes}:{seconds:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DurationMs;

    #[test]
    fn renders_the_way_a_player_shows_it() {
        assert_eq!(DurationMs::from_secs(9).to_string(), "0:09");
        assert_eq!(DurationMs::from_secs(215).to_string(), "3:35");
        assert_eq!(DurationMs::from_secs(3_600).to_string(), "1:00:00");
        assert_eq!(DurationMs::from_secs(4_215).to_string(), "1:10:15");
    }

    #[test]
    fn fraction_is_clamped_and_survives_a_zero_total() {
        let total = DurationMs::from_secs(200);
        assert!((DurationMs::from_secs(50).fraction_of(total) - 0.25).abs() < f32::EPSILON);
        assert!((DurationMs::from_secs(400).fraction_of(total) - 1.0).abs() < f32::EPSILON);
        assert_eq!(DurationMs::from_secs(50).fraction_of(DurationMs::ZERO), 0.0);
    }

    #[test]
    fn subtraction_stops_at_zero() {
        assert_eq!(
            DurationMs::from_secs(1).saturating_sub(DurationMs::from_secs(5)),
            DurationMs::ZERO
        );
    }
}
