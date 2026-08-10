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
            // One-based: the list is read by people, not indexed by machines.
            position: format!("{}", index + 1).into(),
            title: summary.title.as_str().into(),
            artist: summary.artist.as_deref().unwrap_or(UNKNOWN_ARTIST).into(),
            album: summary.album.as_deref().unwrap_or(NO_ALBUM).into(),
            duration: format!("{}", summary.duration).into(),
        })
        .collect()
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

    use super::{NO_ALBUM, UNKNOWN_ARTIST, rows, summary_line};

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

        assert_eq!(rows[0].position, "1");
        assert_eq!(rows[1].position, "2");
        assert_eq!(rows[0].duration, "5:05");
        assert_eq!(rows[1].artist, UNKNOWN_ARTIST, "a missing tag is not blank");
        assert_eq!(rows[1].album, NO_ALBUM);
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
