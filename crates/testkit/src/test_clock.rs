//! A clock the test drives.

use std::sync::atomic::{AtomicI64, Ordering};

use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};

/// The instant an unconfigured [`TestClock`] starts at.
///
/// A fixed, arbitrary point rather than the real time. A test that reads the
/// wall clock passes or fails depending on when it runs, and the retention and
/// history rules are exactly the kind of thing that would then break once a year
/// on somebody else's machine.
pub const DEFAULT_START: Timestamp = Timestamp::from_millis(1_754_611_200_000);

/// A clock that only moves when the test says so.
///
/// Behind an atomic rather than a lock so it can be shared with background
/// threads without a test ever blocking on its own clock.
#[derive(Debug)]
pub struct TestClock {
    millis: AtomicI64,
}

impl TestClock {
    /// A clock stopped at `start`.
    #[must_use]
    pub fn new(start: Timestamp) -> Self {
        Self {
            millis: AtomicI64::new(start.as_millis()),
        }
    }

    /// A clock stopped at a unix-millisecond value.
    #[must_use]
    pub fn at(millis: i64) -> Self {
        Self::new(Timestamp::from_millis(millis))
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: DurationMs) {
        let step = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.millis.fetch_add(step, Ordering::Relaxed);
    }

    /// Moves the clock forward by whole days, for retention tests.
    pub fn advance_days(&self, days: u16) {
        self.advance(DurationMs::from_secs(u64::from(days) * 86_400));
    }

    /// Jumps the clock to an exact instant, forwards or backwards.
    pub fn set(&self, to: Timestamp) {
        self.millis.store(to.as_millis(), Ordering::Relaxed);
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new(DEFAULT_START)
    }
}

impl ClockPort for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.millis.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_START, TestClock};
    use cadenza_core::domain::ports::clock::ClockPort;
    use cadenza_core::domain::value_objects::{DurationMs, Timestamp};

    #[test]
    fn the_clock_stands_still_until_moved() {
        let clock = TestClock::default();
        assert_eq!(clock.now(), DEFAULT_START);
        assert_eq!(clock.now(), DEFAULT_START, "reading it does not advance it");
    }

    #[test]
    fn advancing_moves_it_exactly() {
        let clock = TestClock::default();
        clock.advance(DurationMs::from_secs(90));
        assert_eq!(
            clock.now(),
            DEFAULT_START.saturating_add_millis(90_000),
            "no drift, no rounding"
        );
    }

    #[test]
    fn days_are_whole_days() {
        let clock = TestClock::default();
        clock.advance_days(30);
        assert_eq!(
            clock.now(),
            DEFAULT_START.saturating_add_millis(2_592_000_000)
        );
    }

    #[test]
    fn it_can_be_moved_backwards() {
        let clock = TestClock::default();
        clock.set(Timestamp::UNIX_EPOCH);
        assert_eq!(clock.now(), Timestamp::UNIX_EPOCH);
    }
}
