//! What the decisions screen says about a file it is holding.

use cadenza_core::application::services::ReviewCard;
use cadenza_core::domain::review::ReviewReason;

/// Why a file is waiting, in a sentence rather than a code.
pub fn reason(reason: ReviewReason) -> &'static str {
    match reason {
        ReviewReason::Duplicate => "ALREADY IN YOUR LIBRARY",
        ReviewReason::UnreadableMetadata => "ITS TAGS WOULD NOT READ",
        ReviewReason::UndecodableAudio => "IT WOULD NOT DECODE",
        ReviewReason::MissingFile => "IT WAS GONE BY THE TIME WE LOOKED",
    }
}

/// What it collides with, when it collides with something.
pub fn collides_with(card: &ReviewCard) -> String {
    match &card.existing {
        Some(track) => match &track.artist {
            Some(artist) => format!("the same as {} — {}", artist, track.title),
            None => format!("the same as {}", track.title),
        },
        None => String::new(),
    }
}

/// The line under the title.
pub fn summary_line(waiting: usize) -> String {
    match waiting {
        0 => "nothing is waiting".to_owned(),
        1 => "1 file waiting".to_owned(),
        many => format!("{many} files waiting"),
    }
}

#[cfg(test)]
mod tests {
    use super::{collides_with, reason, summary_line};
    use cadenza_core::application::services::ReviewCard;
    use cadenza_core::domain::ids::{ImportReviewId, MediaFileId};
    use cadenza_core::domain::review::ReviewReason;
    use cadenza_core::domain::track::TrackSummary;
    use cadenza_core::domain::value_objects::DurationMs;

    fn card(existing: Option<TrackSummary>) -> ReviewCard {
        ReviewCard {
            id: ImportReviewId::new(),
            reason: ReviewReason::Duplicate,
            path: None,
            existing,
        }
    }

    fn track(artist: Option<&str>) -> TrackSummary {
        TrackSummary {
            media_file_id: MediaFileId::new(),
            title: "Mysterons".to_owned(),
            artist: artist.map(str::to_owned),
            album: None,
            duration: DurationMs::from_secs(300),
        }
    }

    #[test]
    fn a_reason_is_a_sentence_rather_than_a_code() {
        assert_eq!(reason(ReviewReason::Duplicate), "ALREADY IN YOUR LIBRARY");
        assert_eq!(
            reason(ReviewReason::MissingFile),
            "IT WAS GONE BY THE TIME WE LOOKED"
        );
    }

    #[test]
    fn what_it_collides_with_names_the_track_the_listener_knows() {
        assert_eq!(
            collides_with(&card(Some(track(Some("Portishead"))))),
            "the same as Portishead — Mysterons"
        );
        assert_eq!(
            collides_with(&card(Some(track(None)))),
            "the same as Mysterons"
        );
        assert_eq!(collides_with(&card(None)), "");
    }

    #[test]
    fn the_count_reads_as_words() {
        assert_eq!(summary_line(0), "nothing is waiting");
        assert_eq!(summary_line(1), "1 file waiting");
        assert_eq!(summary_line(4), "4 files waiting");
    }
}
