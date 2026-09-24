//! A point in time, as milliseconds since the unix epoch.
//!
//! Core deliberately owns no calendar library. Every timestamp it handles is a
//! plain integer; rendering to the ISO-8601 `TEXT` form the database stores,
//! and resolving the user's local date for daily aggregates, both belong to
//! infrastructure.
//!
//! Timestamps are never read from the system clock inside core — they arrive
//! through [`crate::domain::ports::clock::ClockPort`], which keeps every
//! time-dependent policy testable.

use std::fmt;

/// Milliseconds in one second.
pub const MILLIS_PER_SECOND: i64 = 1_000;

/// Milliseconds in one day, used by the retention policy.
pub const MILLIS_PER_DAY: i64 = 86_400_000;

/// A point in time, in milliseconds since the unix epoch. May be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(i64);

impl Timestamp {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Self = Self(0);

    /// Wraps a unix-millisecond value.
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Wraps a unix-second value.
    pub const fn from_secs(secs: i64) -> Self {
        Self(secs.saturating_mul(MILLIS_PER_SECOND))
    }

    /// Milliseconds since the unix epoch.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// Moves forward, saturating instead of overflowing.
    pub const fn saturating_add_millis(self, millis: i64) -> Self {
        Self(self.0.saturating_add(millis))
    }

    /// Moves backward, saturating instead of overflowing.
    pub const fn saturating_sub_millis(self, millis: i64) -> Self {
        Self(self.0.saturating_sub(millis))
    }

    /// Moves backward by whole days. Used to compute retention cutoffs.
    pub const fn saturating_sub_days(self, days: u16) -> Self {
        self.saturating_sub_millis((days as i64).saturating_mul(MILLIS_PER_DAY))
    }

    /// Milliseconds elapsed since `earlier`. Negative if `earlier` is in the future.
    pub const fn millis_since(self, earlier: Self) -> i64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{MILLIS_PER_DAY, Timestamp};

    #[test]
    fn day_arithmetic_matches_the_millisecond_constant() {
        let now = Timestamp::from_millis(1_754_611_200_000);
        assert_eq!(
            now.saturating_sub_days(30),
            now.saturating_sub_millis(30 * MILLIS_PER_DAY)
        );
    }

    #[test]
    fn elapsed_time_is_signed() {
        let earlier = Timestamp::from_secs(100);
        let later = Timestamp::from_secs(160);
        assert_eq!(later.millis_since(earlier), 60_000);
        assert_eq!(earlier.millis_since(later), -60_000);
    }

    #[test]
    fn extremes_saturate_rather_than_overflow() {
        assert_eq!(
            Timestamp::from_millis(i64::MIN).saturating_sub_days(u16::MAX),
            Timestamp::from_millis(i64::MIN)
        );
    }
}
