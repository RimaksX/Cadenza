//! Reading the current time.

use crate::domain::value_objects::Timestamp;

/// Supplies the current time.
///
/// Domain code never calls `SystemTime::now()`. Everything time-dependent — the
/// retention cutoff, `created_at`, session timing — takes its reading from here,
/// which is what lets tests pin the clock and assert on exact boundaries instead
/// of sleeping.
pub trait ClockPort: Send + Sync {
    /// The current wall-clock time.
    fn now(&self) -> Timestamp;
}
