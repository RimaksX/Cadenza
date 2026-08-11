//! Turning a library into rows.

use cadenza_core::domain::track::TrackSummary;
use cadenza_core::domain::value_objects::DurationMs;

use crate::TrackRowData;

/// Shown where a file carried no artist tag.
const UNKNOWN_ARTIST: &str = "Unknown artist";

/// Shown where a file carried no album tag.
const NO_ALBUM: &str = "—";

/// Formats the library for display.
///
/// Every value becomes a string here rather than in the markup: a duration
/// format or a fallback for a missing tag is a decision, and decisions made in
/// Rust can be tested.
pub fn rows(summaries: &[TrackSummary]) -> Vec<TrackRowData> {
    summaries
        .iter()
        .enumerate()
        .map(|(index, summary)| TrackRowData {
            id: summary.media_file_id.to_string().into(),
            position: position(index).into(),
            title: summary.title.as_str().into(),
            artist: summary.artist.as_deref().unwrap_or(UNKNOWN_ARTIST).into(),
            album: summary.album.as_deref().unwrap_or(NO_ALBUM).into(),
            duration: clock(summary.duration).into(),
        })
        .collect()
}

/// The number in the left column.
///
/// One-based, because the list is read by people rather than indexed by
/// machines, and padded to two digits so a column of them is a column: `9`
/// above `10` puts a ragged edge in the quietest part of the row. Past 99 the
/// number simply grows — padding further would widen every row in the library
/// for the sake of the last few.
fn position(index: usize) -> String {
    format!("{:02}", index + 1)
}

/// A duration in a column of durations.
///
/// `mm:ss` with both parts padded, and hours only when there are any. The
/// player bar uses the domain's own `m:ss` instead: one readout reads better
/// unpadded, a column reads better aligned.
fn clock(duration: DurationMs) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3_600, (total % 3_600) / 60, total % 60);

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The line under the page title: how much there is, and how long it runs.
pub fn summary_line(summaries: &[TrackSummary]) -> String {
    if summaries.is_empty() {
        return "no tracks yet".to_owned();
    }

    let total = summaries.iter().fold(DurationMs::ZERO, |sum, summary| {
        sum.saturating_add(summary.duration)
    });

    let count = summaries.len();
    let noun = if count == 1 { "track" } else { "tracks" };
    format!("{count} {noun} · {total}")
}

#[cfg(test)]
mod tests {
    use cadenza_core::domain::ids::MediaFileId;
    use cadenza_core::domain::track::TrackSummary;
    use cadenza_core::domain::value_objects::DurationMs;

    use super::{NO_ALBUM, UNKNOWN_ARTIST, clock, rows, summary_line};

    fn summary(title: &str, artist: Option<&str>, seconds: u64) -> TrackSummary {
        TrackSummary {
            media_file_id: MediaFileId::new(),
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            album: None,
            duration: DurationMs::from_secs(seconds),
        }
    }

    #[test]
    fn rows_are_numbered_from_one_and_formatted() {
        let library = [
            summary("Mysterons", Some("Portishead"), 305),
            summary("Teardrop", None, 330),
        ];
        let rows = rows(&library);

        assert_eq!(rows[0].position, "01");
        assert_eq!(rows[1].position, "02");
        assert_eq!(rows[0].duration, "05:05");
        assert_eq!(rows[1].artist, UNKNOWN_ARTIST, "a missing tag is not blank");
        assert_eq!(rows[1].album, NO_ALBUM);
    }

    #[test]
    fn times_in_a_column_are_padded_so_the_column_is_straight() {
        assert_eq!(clock(DurationMs::from_secs(9)), "00:09");
        assert_eq!(clock(DurationMs::from_secs(65)), "01:05");
        assert_eq!(clock(DurationMs::from_secs(600)), "10:00");
        assert_eq!(
            clock(DurationMs::from_secs(3_930)),
            "1:05:30",
            "hours only when there are any"
        );
    }

    #[test]
    fn the_summary_counts_and_adds_up() {
        let library = [
            summary("Mysterons", Some("Portishead"), 300),
            summary("Teardrop", Some("Massive Attack"), 330),
        ];
        assert_eq!(summary_line(&library), "2 tracks · 10:30");
    }

    #[test]
    fn one_track_is_not_one_tracks() {
        let library = [summary("Mysterons", None, 60)];
        assert_eq!(summary_line(&library), "1 track · 1:00");
    }

    #[test]
    fn an_empty_library_says_so_rather_than_showing_a_zero() {
        assert_eq!(summary_line(&[]), "no tracks yet");
    }
}
