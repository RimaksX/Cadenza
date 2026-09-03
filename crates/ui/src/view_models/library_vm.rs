//! Turning a library into rows.

use cadenza_core::application::services::ScanReport;
use cadenza_core::domain::ports::fetcher::MissingTool;
use cadenza_core::domain::track::TrackSummary;
use cadenza_core::domain::value_objects::DurationMs;

use crate::TrackRowData;
use crate::view_models::clock;

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

/// The tracks a search finds.
///
/// Case-insensitive, and across title, artist and album together rather than
/// one field at a time: somebody typing "portishead" is looking for the artist,
/// and somebody typing "dummy" for the record, and neither of them wants to
/// pick which box to type it into first. An empty query matches everything.
pub fn matching(summaries: &[TrackSummary], query: &str) -> Vec<TrackSummary> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return summaries.to_vec();
    }

    summaries
        .iter()
        .filter(|summary| {
            let haystack = format!(
                "{} {} {}",
                summary.title,
                summary.artist.as_deref().unwrap_or_default(),
                summary.album.as_deref().unwrap_or_default()
            )
            .to_lowercase();

            // Every word has to appear somewhere: "portishead mys" finds one
            // track rather than nothing, which is how people actually narrow a
            // list down.
            needle
                .split_whitespace()
                .all(|word| haystack.contains(word))
        })
        .cloned()
        .collect()
}

/// The line under the page title while a search is running.
pub fn found_line(shown: &[TrackSummary], query: &str, total: usize) -> String {
    if query.trim().is_empty() {
        return summary_line(shown);
    }
    if shown.is_empty() {
        return format!("nothing of {total} matches");
    }

    let total_time = shown.iter().fold(DurationMs::ZERO, |sum, summary| {
        sum.saturating_add(summary.duration)
    });
    format!("{} of {total} · {total_time}", shown.len())
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

    use super::{NO_ALBUM, UNKNOWN_ARTIST, found_line, matching, rows, summary_line};

    #[test]
    fn a_search_looks_at_the_title_the_artist_and_the_album_together() {
        let library = [
            summary("Mysterons", Some("Portishead"), 305),
            summary("Teardrop", Some("Massive Attack"), 330),
        ];

        assert_eq!(matching(&library, "portis").len(), 1);
        assert_eq!(matching(&library, "TEAR").len(), 1, "case does not matter");
        assert_eq!(
            matching(&library, "").len(),
            2,
            "an empty query is not a filter"
        );
        assert_eq!(matching(&library, "   ").len(), 2);
    }

    #[test]
    fn every_word_of_a_search_has_to_land_somewhere() {
        let library = [
            summary("Mysterons", Some("Portishead"), 305),
            summary("Teardrop", Some("Massive Attack"), 330),
        ];

        assert_eq!(
            matching(&library, "portishead mys").len(),
            1,
            "words narrow rather than widen"
        );
        assert!(matching(&library, "portishead teardrop").is_empty());
    }

    #[test]
    fn the_page_line_says_how_much_of_the_library_answered() {
        let library = [
            summary("Mysterons", Some("Portishead"), 300),
            summary("Teardrop", Some("Massive Attack"), 330),
        ];
        let found = matching(&library, "portis");

        assert_eq!(found_line(&found, "portis", library.len()), "1 of 2 · 5:00");
        assert_eq!(
            found_line(&[], "zzz", library.len()),
            "nothing of 2 matches"
        );
        assert_eq!(
            found_line(&library, "", library.len()),
            summary_line(&library),
            "with no search it is the ordinary count"
        );
    }

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

/// What a drop did, in the words the player bar has room for.
///
/// Named counts rather than a total, because the four outcomes mean different
/// things to somebody who has just let go of a handful of files: what arrived,
/// what was already here, and what could not be read and is therefore waiting
/// on the decisions screen.
pub fn taken_in(report: &ScanReport) -> String {
    let mut said = Vec::new();

    if report.added > 0 {
        said.push(format!("{} {} added", report.added, tracks(report.added)));
    }
    if report.updated > 0 {
        said.push(format!("{} re-read", report.updated));
    }
    let known = report.duplicates + report.unchanged;
    if known > 0 {
        said.push(format!("{known} already here"));
    }
    if report.gone > 0 {
        said.push(format!("{} gone from the folder", report.gone));
    }
    if report.failed > 0 {
        said.push(format!(
            "{} could not be read — see Decisions",
            report.failed
        ));
    }

    if said.is_empty() {
        return "nothing there to add".to_owned();
    }
    said.join(" · ")
}

/// What is missing before a link can be fetched, as one line.
///
/// Names the programs and says what each is for. Somebody who has neither
/// should be able to read this once and know what to go and install; a line
/// that said only "the tools are missing" would send them to a search engine
/// to find out which tools.
pub fn tools_needed(missing: &[MissingTool]) -> String {
    let named: Vec<String> = missing
        .iter()
        .map(|tool| format!("{} ({})", tool.name, tool.reason))
        .collect();

    format!(
        "a link needs {} — install {} and press GET again",
        if named.len() == 1 {
            "one more program"
        } else {
            "two more programs"
        },
        named.join(" and ")
    )
}

/// "track" or "tracks", so a count reads as a sentence.
fn tracks(count: usize) -> &'static str {
    if count == 1 { "track" } else { "tracks" }
}

#[cfg(test)]
mod drop_tests {
    use super::taken_in;
    use cadenza_core::application::services::ScanReport;

    #[test]
    fn a_drop_says_what_it_did() {
        assert_eq!(
            taken_in(&ScanReport {
                seen: 1,
                added: 1,
                ..ScanReport::default()
            }),
            "1 track added"
        );

        assert_eq!(
            taken_in(&ScanReport {
                seen: 3,
                added: 2,
                duplicates: 1,
                ..ScanReport::default()
            }),
            "2 tracks added · 1 already here"
        );

        // Dropping something that is not music at all, or a folder with no
        // music in it: an answer, not silence.
        assert_eq!(taken_in(&ScanReport::default()), "nothing there to add");

        assert_eq!(
            taken_in(&ScanReport {
                seen: 1,
                failed: 1,
                ..ScanReport::default()
            }),
            "1 could not be read — see Decisions"
        );
    }
}
