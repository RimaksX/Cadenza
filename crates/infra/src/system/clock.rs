//! The wall clock.

use std::time::{SystemTime, UNIX_EPOCH};

use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::value_objects::Timestamp;

/// Reads the operating system clock.
///
/// The only implementation that does. Everything else — retention cutoffs,
/// `created_at`, session timing — goes through [`ClockPort`], which is what lets
/// a test pin the clock and assert on an exact boundary instead of sleeping.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl ClockPort for SystemClock {
    fn now(&self) -> Timestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since_epoch) => Timestamp::from_millis(clamp_millis(since_epoch.as_millis())),
            // The machine's clock is set before 1970. Absurd, but a wrong sign
            // is better than a panic in something this fundamental.
            Err(before_epoch) => {
                Timestamp::from_millis(-clamp_millis(before_epoch.duration().as_millis()))
            }
        }
    }
}

/// Narrows a `u128` millisecond count into the `i64` a [`Timestamp`] holds.
///
/// Saturating rather than wrapping: a clock reading past the year 292 million is
/// broken hardware, and wrapping would silently turn it into a date in the past.
fn clamp_millis(millis: u128) -> i64 {
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{SystemClock, clamp_millis};
    use cadenza_core::domain::ports::clock::ClockPort;
    use cadenza_core::domain::value_objects::Timestamp;

    #[test]
    fn the_clock_reports_a_plausible_present() {
        // 2020-01-01, comfortably in the past for any machine running this.
        let year_2020 = Timestamp::from_millis(1_577_836_800_000);
        assert!(
            SystemClock.now() > year_2020,
            "the system clock should be somewhere after 2020"
        );
    }

    #[test]
    fn an_impossible_reading_saturates_rather_than_wrapping() {
        assert_eq!(clamp_millis(u128::MAX), i64::MAX);
        assert_eq!(clamp_millis(1_754_611_200_000), 1_754_611_200_000);
    }
}
