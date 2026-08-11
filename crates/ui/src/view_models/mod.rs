//! Translation between the application layer and the markup.
//!
//! A view model formats and nothing else. It does not decide what may be
//! played, what happens on a click, or what the library contains — those are
//! the application layer's business (PROJECT_MASTER 4.3). It decides that a
//! missing artist reads "Unknown artist" and that five minutes reads "5:00".
//!
//! Keeping that here rather than in Slint is what makes it testable: these are
//! plain functions over plain data, and the tests below need no window.

pub mod library_vm;
pub mod player_vm;

use cadenza_core::domain::value_objects::DurationMs;

/// A duration as the interface writes it: `mm:ss`, with hours only when there
/// are any.
///
/// Both parts padded, everywhere. A time under a progress line and a column of
/// times in a listing are read the same way, and a format that changes width as
/// the minute rolls over makes the eye re-find it.
pub(crate) fn clock(duration: DurationMs) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3_600, (total % 3_600) / 60, total % 60);

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::value_objects::DurationMs;

    use super::clock;

    #[test]
    fn times_are_padded_so_a_column_of_them_is_straight() {
        assert_eq!(clock(DurationMs::from_secs(9)), "00:09");
        assert_eq!(clock(DurationMs::from_secs(65)), "01:05");
        assert_eq!(clock(DurationMs::from_secs(600)), "10:00");
        assert_eq!(
            clock(DurationMs::from_secs(3_930)),
            "1:05:30",
            "hours only when there are any"
        );
    }
}
