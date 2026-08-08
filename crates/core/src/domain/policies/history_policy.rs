//! How a listen is classified, and whether it is recorded at all.

use crate::domain::profile::Profile;
use crate::domain::stats::PlayOutcome;
use crate::domain::value_objects::DurationMs;

/// A listen counts as complete once more than this fraction has played.
///
/// PROJECT_MASTER 2.6: "более 50%".
pub const COMPLETION_FRACTION: f32 = 0.5;

/// Switching away within this window counts as a skip.
///
/// PROJECT_MASTER 2.6: "до 10 секунд".
pub const SKIP_WINDOW: DurationMs = DurationMs::from_secs(10);

/// Classifies how a listen ended.
///
/// # Resolving the short-track conflict
///
/// The two rules in PROJECT_MASTER 2.6 overlap for tracks shorter than twenty
/// seconds: an interlude of 15 s stopped at 9 s is both "more than 50% played"
/// and "switched away before 10 seconds". The specification does not say which
/// wins.
///
/// Completion wins here. The listener heard most of the track, and the skip rule
/// exists to catch "this is not what I wanted" — which is not what happened. The
/// alternative would make every short track count as a rejection and would
/// poison both the skip rate and radio's negative feedback with interludes and
/// intros.
pub fn classify(played: DurationMs, total: DurationMs) -> PlayOutcome {
    if !total.is_zero() && played.as_millis().saturating_mul(2) > total.as_millis() {
        return PlayOutcome::Completed;
    }
    if played < SKIP_WINDOW {
        return PlayOutcome::Skipped;
    }
    PlayOutcome::Partial
}

/// Whether a listening event may be written at all.
///
/// When history is off, nothing is stored — not a reduced record, not an
/// anonymised one (PROJECT_MASTER 1.4, 2.6). Every write path checks this first.
pub const fn should_record(profile: &Profile) -> bool {
    profile.history_enabled
}

#[cfg(test)]
mod tests {
    use super::{SKIP_WINDOW, classify, should_record};
    use crate::domain::profile::{Profile, ProfileName};
    use crate::domain::stats::PlayOutcome;
    use crate::domain::value_objects::{DurationMs, Timestamp};

    const FOUR_MINUTES: DurationMs = DurationMs::from_secs(240);

    #[test]
    fn more_than_half_is_a_completed_listen() {
        assert_eq!(
            classify(DurationMs::from_secs(121), FOUR_MINUTES),
            PlayOutcome::Completed
        );
    }

    #[test]
    fn exactly_half_is_not_yet_complete() {
        assert_eq!(
            classify(DurationMs::from_secs(120), FOUR_MINUTES),
            PlayOutcome::Partial,
            "the rule says more than 50%, not at least 50%"
        );
    }

    #[test]
    fn switching_away_early_is_a_skip() {
        assert_eq!(
            classify(DurationMs::from_secs(4), FOUR_MINUTES),
            PlayOutcome::Skipped
        );
        assert_eq!(
            classify(
                SKIP_WINDOW.saturating_sub(DurationMs::from_millis(1)),
                FOUR_MINUTES
            ),
            PlayOutcome::Skipped
        );
    }

    #[test]
    fn the_window_boundary_is_not_a_skip() {
        assert_eq!(
            classify(SKIP_WINDOW, FOUR_MINUTES),
            PlayOutcome::Partial,
            "the rule says before 10 seconds"
        );
    }

    #[test]
    fn the_middle_band_counts_as_neither() {
        assert_eq!(
            classify(DurationMs::from_secs(60), FOUR_MINUTES),
            PlayOutcome::Partial
        );
    }

    #[test]
    fn on_a_short_track_completion_beats_the_skip_window() {
        let interlude = DurationMs::from_secs(15);
        assert_eq!(
            classify(DurationMs::from_secs(9), interlude),
            PlayOutcome::Completed,
            "9s of a 15s track is 60% heard, not a rejection"
        );
        assert_eq!(
            classify(DurationMs::from_secs(2), interlude),
            PlayOutcome::Skipped,
            "but genuinely bailing out early is still a skip"
        );
    }

    #[test]
    fn an_unknown_duration_falls_back_to_the_skip_window() {
        assert_eq!(
            classify(DurationMs::from_secs(3), DurationMs::ZERO),
            PlayOutcome::Skipped
        );
        assert_eq!(
            classify(DurationMs::from_secs(90), DurationMs::ZERO),
            PlayOutcome::Partial
        );
    }

    #[test]
    fn nothing_is_recorded_when_history_is_off() {
        let mut profile = Profile::new(
            ProfileName::new("Sasha").expect("valid"),
            Timestamp::UNIX_EPOCH,
        );
        assert!(!should_record(&profile), "history is off by default");

        profile.history_enabled = true;
        assert!(should_record(&profile));
    }
}
