//! How hard background analysis is allowed to work.
//!
//! The rule is stated as a number — background work stays
//! under roughly a fifth of the machine — and it is an absolute:
//! every background task is low priority and never blocks. Both are here, as a
//! share of wall-clock time the worker is permitted to spend running.
//!
//! A share rather than a thread priority because a share is the thing that can
//! be reasoned about and tested. Lowering a thread's priority asks the operating
//! system for a favour; resting for four times as long as you worked is a
//! promise you keep yourself.

use core::time::Duration;

/// Where feature extraction sits against the other kinds of background work.
///
/// Below both of them, and deliberately: the library is unusable without titles
/// and merely less clever without BPM.
pub const FEATURES_PRIORITY: i32 = 0;

/// How many files are queued for analysis in one top-up.
///
/// Small enough that a listener who opens Cadenza for one song is not paying
/// for a five-thousand-row insert, large enough that the worker is not asking
/// the database for more work every few seconds.
pub const TOP_UP_BATCH: usize = 64;

/// The share of the machine analysis may take while music is playing.
///
/// A fifth of one core is the whole allowance, and while
/// something is playing the listener has a use for the rest of it.
pub const SHARE_WHILE_PLAYING: f32 = 0.2;

/// The share it may take while nothing is playing.
///
/// Higher, because there is nothing to disturb: the point of analysing at all
/// is that the answers are ready before anybody asks for them. Still well short
/// of the whole core, so the machine stays somebody else's to use.
pub const SHARE_WHEN_IDLE: f32 = 0.5;

/// The longest single rest, so that a stop is noticed promptly.
///
/// The worker checks whether it has been told to stop between naps, and a nap
/// measured in minutes would keep the process alive long after the window
/// closed.
pub const MAX_NAP: Duration = Duration::from_secs(2);

/// How long to rest after working for `worked`, to stay within `share`.
///
/// Working `w` out of every `w + r` means a share of `w / (w + r)`, so the rest
/// that holds a share is `w * (1 - share) / share`. One file takes about a
/// second, so at a fifth the worker sleeps about four.
pub fn nap_after(worked: Duration, share: f32) -> Duration {
    // A share at or above one is "take what you like"; at or below zero the
    // worker would never run again, and a worker that never runs is a bug
    // waiting to be reported as "analysis is broken".
    if !share.is_finite() || share >= 1.0 {
        return Duration::ZERO;
    }
    let share = share.max(0.05);

    let rest = worked.mul_f32((1.0 - share) / share);
    rest.min(MAX_NAP)
}

#[cfg(test)]
mod tests {
    use super::{MAX_NAP, SHARE_WHEN_IDLE, SHARE_WHILE_PLAYING, nap_after};
    use core::time::Duration;

    #[test]
    fn resting_four_times_the_work_is_a_fifth_of_the_machine() {
        let worked = Duration::from_millis(100);
        let nap = nap_after(worked, SHARE_WHILE_PLAYING);

        assert!(
            nap.abs_diff(Duration::from_millis(400)) < Duration::from_millis(1),
            "four times the work, give or take what f32 can hold: {nap:?}"
        );

        let cycle = worked + nap;
        let share = worked.as_secs_f32() / cycle.as_secs_f32();
        assert!((share - SHARE_WHILE_PLAYING).abs() < 0.001);
    }

    #[test]
    fn an_idle_machine_is_worked_harder() {
        let worked = Duration::from_millis(100);
        assert!(nap_after(worked, SHARE_WHEN_IDLE) < nap_after(worked, SHARE_WHILE_PLAYING));
    }

    #[test]
    fn a_long_file_does_not_buy_a_long_sleep() {
        assert_eq!(nap_after(Duration::from_secs(60), 0.2), MAX_NAP);
    }

    #[test]
    fn nonsense_shares_do_not_stall_the_worker() {
        assert_eq!(nap_after(Duration::from_millis(10), 1.0), Duration::ZERO);
        assert_eq!(
            nap_after(Duration::from_millis(10), f32::NAN),
            Duration::ZERO
        );

        // Zero would mean "never run again". It is treated as "run rarely".
        let nap = nap_after(Duration::from_millis(10), 0.0);
        assert!(nap > Duration::ZERO && nap <= MAX_NAP);
    }
}
