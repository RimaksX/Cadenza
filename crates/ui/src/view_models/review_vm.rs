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
///
/// It says why they are waiting as well as how many, because the label that
/// used to say so is gone: an eyebrow over every page named the page, and this
/// was the one whose words carried a fact rather than a name
/// . A count on its own would leave "waiting for what?"
/// with nowhere to be answered.
pub fn summary_line(waiting: usize) -> String {
    match waiting {
        0 => "nothing is held back".to_owned(),
        1 => "1 file held back rather than imported".to_owned(),
        many => format!("{many} files held back rather than imported"),
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
    fn the_count_reads_as_words_and_says_what_kind_of_waiting() {
        // "Waiting" alone left the obvious question unanswered once the label
        // above it went.
        assert_eq!(summary_line(0), "nothing is held back");
        assert_eq!(summary_line(1), "1 file held back rather than imported");
        assert_eq!(summary_line(4), "4 files held back rather than imported");
    }
}
