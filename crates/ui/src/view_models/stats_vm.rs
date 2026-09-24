//! What the listening screen shows.

use crate::text::{plural, tr, tr1};
use cadenza_core::application::services::ListeningReport;
use cadenza_core::domain::value_objects::DurationMs;

/// How long was heard, in the largest unit that stays legible.
///
/// Hours once there is an hour of it. A listener who played four thousand
/// minutes last month should not have to divide.
pub fn heard(listened: DurationMs) -> String {
    let minutes = listened.as_millis() / 60_000;
    // Templates rather than letters glued to numbers: Russian writes the unit
    // as a word after a space, and English as a letter against the digit.
    if minutes < 60 {
        return tr("{m}m").replace("{m}", &minutes.to_string());
    }
    tr("{h}h {m}m")
        .replace("{h}", &(minutes / 60).to_string())
        .replace("{m}", &(minutes % 60).to_string())
}

/// The share of listens abandoned early, as a percentage.
pub fn skip_rate(report: &ListeningReport) -> String {
    format!("{:.0}%", report.summary.skip_rate() * 100.0)
}

/// The line under the title.
pub fn summary_line(report: &ListeningReport) -> String {
    if !report.keeping {
        return tr("history is off - nothing is written down").to_owned();
    }
    if report.summary.started == 0 {
        return tr1(
            "nothing played in the last {} days",
            &report.days.to_string(),
        );
    }

    let noun = plural(
        u64::from(report.summary.started),
        "start|one",
        "starts|few",
        "starts|many",
    );
    format!(
        "{} {noun} {} {} {}",
        report.summary.started,
        tr("over|span"),
        report.days,
        plural(u64::from(report.days), "day|one", "days|few", "days|many"),
    )
}

/// How many times a track was played through.
pub fn plays(count: u32) -> String {
    let noun = plural(u64::from(count), "play|one", "plays|few", "plays|many");
    format!("{count} {noun}")
}

#[cfg(test)]
mod tests {
    use super::{heard, plays, skip_rate, summary_line};
    use cadenza_core::application::services::ListeningReport;
    use cadenza_core::domain::stats::ListeningSummary;
    use cadenza_core::domain::value_objects::DurationMs;

    fn report(started: u32, skipped: u32, keeping: bool) -> ListeningReport {
        ListeningReport {
            days: 30,
            summary: ListeningSummary {
                started,
                completed: started - skipped,
                skipped,
                tracks: started,
                listened: DurationMs::from_secs(600),
            },
            top: Vec::new(),
            keeping,
        }
    }

    #[test]
    fn time_reads_in_the_unit_that_suits_it() {
        assert_eq!(heard(DurationMs::from_secs(90)), "1m");
        assert_eq!(heard(DurationMs::from_secs(59 * 60)), "59m");
        assert_eq!(heard(DurationMs::from_secs(60 * 60)), "1h 0m");
        assert_eq!(heard(DurationMs::from_secs(2 * 60 * 60 + 5 * 60)), "2h 5m");
    }

    #[test]
    fn the_line_says_which_kind_of_empty_this_is() {
        assert_eq!(
            summary_line(&report(0, 0, false)),
            "history is off - nothing is written down"
        );
        assert_eq!(
            summary_line(&report(0, 0, true)),
            "nothing played in the last 30 days"
        );
        assert_eq!(summary_line(&report(1, 0, true)), "1 start over 30 days");
        assert_eq!(summary_line(&report(9, 0, true)), "9 starts over 30 days");
    }

    #[test]
    fn rates_and_counts_read_as_words_where_they_should() {
        assert_eq!(skip_rate(&report(4, 1, true)), "25%");
        assert_eq!(skip_rate(&report(0, 0, true)), "0%");
        assert_eq!(plays(1), "1 play");
        assert_eq!(plays(12), "12 plays");
    }
}
