//! How long listening history survives.

use crate::domain::profile::HISTORY_RETENTION_DAYS;
use crate::domain::value_objects::Timestamp;

/// The oldest timestamp worth keeping.
///
/// `retention_days` is capped at [`HISTORY_RETENTION_DAYS`]: a stored value
/// larger than the policy allows is treated as the policy maximum rather than
/// honoured, so a corrupted or hand-edited row cannot quietly extend how long
/// personal data is kept.
pub fn cutoff(now: Timestamp, retention_days: u16) -> Timestamp {
    now.saturating_sub_days(retention_days.min(HISTORY_RETENTION_DAYS))
}

/// True when an event is old enough to purge.
///
/// Events exactly on the cutoff are kept; only strictly older ones go.
pub fn is_expired(recorded_at: Timestamp, now: Timestamp, retention_days: u16) -> bool {
    recorded_at < cutoff(now, retention_days)
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_RETENTION_DAYS, cutoff, is_expired};
    use crate::domain::value_objects::Timestamp;

    const NOW: Timestamp = Timestamp::from_millis(1_754_611_200_000);

    #[test]
    fn events_older_than_the_window_expire() {
        let thirty_one_days_ago = NOW.saturating_sub_days(31);
        let yesterday = NOW.saturating_sub_days(1);

        assert!(is_expired(thirty_one_days_ago, NOW, 30));
        assert!(!is_expired(yesterday, NOW, 30));
        assert!(!is_expired(NOW, NOW, 30));
    }

    #[test]
    fn the_boundary_event_is_kept() {
        let exactly_thirty_days_ago = NOW.saturating_sub_days(30);
        assert!(!is_expired(exactly_thirty_days_ago, NOW, 30));
    }

    #[test]
    fn a_shorter_window_is_honoured() {
        let ten_days_ago = NOW.saturating_sub_days(10);
        assert!(is_expired(ten_days_ago, NOW, 7));
        assert!(!is_expired(ten_days_ago, NOW, 30));
    }

    #[test]
    fn a_longer_window_cannot_extend_retention() {
        assert_eq!(
            cutoff(NOW, 3_650),
            cutoff(NOW, HISTORY_RETENTION_DAYS),
            "a stored value above the policy cap must not widen the window"
        );
        assert!(is_expired(NOW.saturating_sub_days(60), NOW, 3_650));
    }
}
