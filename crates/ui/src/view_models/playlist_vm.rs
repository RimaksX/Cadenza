//! Turning playlists into rows.

use cadenza_core::application::services::PlaylistSummary;

use crate::PlaylistRowData;

/// Formats the index of playlists.
pub fn rows(summaries: &[PlaylistSummary]) -> Vec<PlaylistRowData> {
    summaries
        .iter()
        .map(|summary| PlaylistRowData {
            id: summary.playlist.id.to_string().into(),
            name: summary.playlist.name.as_str().into(),
            summary: count(summary.track_count).into(),
        })
        .collect()
}

/// How much is in a list, in words.
///
/// "Empty" rather than "0 tracks": a list with nothing in it is a state, not a
/// measurement, and it is the one the listener has to act on.
fn count(tracks: usize) -> String {
    match tracks {
        0 => "empty".to_owned(),
        1 => "1 track".to_owned(),
        many => format!("{many} tracks"),
    }
}

#[cfg(test)]
mod tests {
    use cadenza_core::application::services::PlaylistSummary;
    use cadenza_core::domain::ids::{PlaylistId, ProfileId};
    use cadenza_core::domain::playlist::Playlist;
    use cadenza_core::domain::value_objects::Timestamp;

    use super::{count, rows};

    fn summary(name: &str, track_count: usize) -> PlaylistSummary {
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
        }
    }

    #[test]
    fn a_row_carries_its_name_and_its_size() {
        let listed = rows(&[summary("Late night", 12)]);
        assert_eq!(listed[0].name, "Late night");
        assert_eq!(listed[0].summary, "12 tracks");
        assert!(
            !listed[0].id.is_empty(),
            "the row hands its id back on click"
        );
    }

    #[test]
    fn an_empty_list_says_so_rather_than_counting_to_zero() {
        assert_eq!(count(0), "empty");
        assert_eq!(count(1), "1 track", "and one is not one tracks");
    }
}
