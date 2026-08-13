//! Turning playlists into cards.

use cadenza_core::application::services::PlaylistSummary;
use cadenza_core::domain::value_objects::DurationMs;

use crate::{MenuItemData, PlaylistCardData};

/// Formats the index of playlists.
pub fn cards(summaries: &[PlaylistSummary]) -> Vec<PlaylistCardData> {
    summaries
        .iter()
        .enumerate()
        .map(|(index, summary)| PlaylistCardData {
            id: summary.playlist.id.to_string().into(),
            name: summary.playlist.name.as_str().into(),
            // The tile's corner marks, in the mono capitals everything numeric
            // is set in. Upper-cased here rather than in the markup: Slint has
            // no text-transform, so a label that must read as capitals has to
            // arrive as capitals.
            position: format!("№ {:02}", index + 1).into(),
            // What the album tile puts here is the artist. A playlist has no
            // artist; what it has is whose list it is.
            maker: "MADE BY YOU".into(),
            meta: meta(summary).into(),
        })
        .collect()
}

/// The playlists as a list of destinations to pick from.
///
/// The same struct the menus use, because it is the same thing: a label and
/// what the interface hands back when it is chosen.
pub fn options(summaries: &[PlaylistSummary]) -> Vec<MenuItemData> {
    summaries
        .iter()
        .map(|summary| MenuItemData {
            action: summary.playlist.id.to_string().into(),
            label: summary.playlist.name.as_str().into(),
            meta: meta(summary).into(),
            destructive: false,
        })
        .collect()
}

/// The line under the tile: how many tracks, and how long they run.
fn meta(summary: &PlaylistSummary) -> String {
    format!(
        "{} {} · {}",
        summary.track_count,
        noun(summary.track_count).to_uppercase(),
        duration(summary)
    )
}

/// The noun that goes with the count.
fn noun(tracks: usize) -> &'static str {
    if tracks == 1 { "track" } else { "tracks" }
}

/// How long a playlist runs, or a dash when there is nothing in it.
///
/// A dash rather than "0:00": an empty list has no length, and a zero invites
/// the reader to wonder whether the tracks are all silent.
fn duration(summary: &PlaylistSummary) -> String {
    if summary.track_count == 0 {
        return "—".to_owned();
    }
    summary.duration.to_string()
}

/// The line under the page title: how many lists, and how much is in them.
pub fn summary_line(summaries: &[PlaylistSummary]) -> String {
    if summaries.is_empty() {
        return "no lists yet".to_owned();
    }

    let lists = summaries.len();
    let tracks: usize = summaries.iter().map(|summary| summary.track_count).sum();
    let total = summaries.iter().fold(DurationMs::ZERO, |sum, summary| {
        sum.saturating_add(summary.duration)
    });

    format!(
        "{lists} {} · {tracks} {} · {total}",
        if lists == 1 { "list" } else { "lists" },
        noun(tracks)
    )
}

#[cfg(test)]
mod tests {
    use cadenza_core::application::services::PlaylistSummary;
    use cadenza_core::domain::ids::{PlaylistId, ProfileId};
    use cadenza_core::domain::playlist::Playlist;
    use cadenza_core::domain::value_objects::{DurationMs, Timestamp};

    use super::{cards, duration, meta, noun, summary_line};

    fn summary(name: &str, track_count: usize, seconds: u64) -> PlaylistSummary {
        PlaylistSummary {
            playlist: Playlist {
                id: PlaylistId::new(),
                profile_id: ProfileId::new(),
                name: name.to_owned(),
                description: None,
                is_smart: false,
                rule_json: None,
                created_at: Timestamp::from_millis(0),
                updated_at: Timestamp::from_millis(0),
            },
            track_count,
            duration: DurationMs::from_secs(seconds),
        }
    }

    #[test]
    fn a_card_carries_its_name_its_place_and_what_is_in_it() {
        let listed = cards(&[summary("Late night", 12, 2_640), summary("Morning", 3, 600)]);
        assert_eq!(listed[0].name, "Late night");
        assert_eq!(
            listed[0].position, "№ 01",
            "padded, so a column of them is one"
        );
        assert_eq!(listed[1].position, "№ 02");
        assert_eq!(listed[0].meta, "12 TRACKS · 44:00");
        assert!(
            !listed[0].id.is_empty(),
            "the card hands its id back on click"
        );
    }

    #[test]
    fn one_track_reads_as_one_track_on_the_tile_too() {
        assert_eq!(meta(&summary("Single", 1, 300)), "1 TRACK · 5:00");
    }

    #[test]
    fn one_track_is_not_one_tracks() {
        assert_eq!(noun(1), "track");
        assert_eq!(noun(0), "tracks");
    }

    #[test]
    fn an_empty_list_has_no_length_rather_than_a_zero_one() {
        assert_eq!(duration(&summary("New", 0, 0)), "—");
    }

    #[test]
    fn the_page_line_counts_lists_and_what_is_in_them() {
        let listed = [summary("Late night", 12, 2_640), summary("Morning", 3, 600)];
        assert_eq!(summary_line(&listed), "2 lists · 15 tracks · 54:00");
        assert_eq!(summary_line(&listed[..1]), "1 list · 12 tracks · 44:00");
        assert_eq!(summary_line(&[]), "no lists yet");
    }

    #[test]
    fn an_empty_list_shows_a_dash_where_its_length_would_go() {
        assert_eq!(meta(&summary("New", 0, 0)), "0 TRACKS · —");
    }
}
