//! Turning the player's state into what the bar shows.

use cadenza_core::application::view_state::PlayerView;
use cadenza_core::domain::track::TrackSummary;

use crate::view_models::{clock, initial};

/// What the player bar draws, as strings and numbers the markup can bind.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerFields {
    /// Title of the loaded track.
    pub title: String,
    /// Artist and album on one line, as a caption under the title.
    pub subtitle: String,
    /// The letter standing in for cover art.
    pub initial: String,
    /// Elapsed time.
    pub position: String,
    /// Total time.
    pub duration: String,
    /// Identifier of the loaded track, so the listing can mark its row.
    pub playing_id: String,
    /// Progress, `0.0..=1.0`.
    pub progress: f32,
    /// Whether audio is running.
    pub playing: bool,
    /// Whether a track is loaded at all.
    pub loaded: bool,
    /// Whether output is silenced.
    pub muted: bool,
    /// The level the slider shows.
    pub volume: f32,
}

/// Projects the player's view state.
pub fn fields(view: &PlayerView) -> PlayerFields {
    let track = view.track.as_ref();

    PlayerFields {
        title: track
            .map(|summary| summary.title.clone())
            .unwrap_or_default(),
        subtitle: track.map(subtitle).unwrap_or_default(),
        initial: track
            .map(|summary| initial(&summary.title))
            .unwrap_or_default(),
        position: clock(view.position.elapsed()),
        duration: clock(view.duration),
        playing_id: track
            .map(|summary| summary.media_file_id.to_string())
            .unwrap_or_default(),
        progress: view.progress(),
        playing: view.state.is_playing(),
        // A finished track stays loaded as far as the bar is concerned: its
        // title is still what was last heard, and pressing play starts it
        // again. What is *not* loaded is a player that has never been given
        // anything, or one that was stopped.
        loaded: track.is_some(),
        muted: view.muted,
        volume: view.volume.as_f32(),
    }
}

/// Artist and album on one line, joined only when both are there.
///
/// A separator with nothing after it reads as a truncation, and an untagged
/// file is common enough that it must not look like a bug.
///
/// Set in capitals, like every other mono caption in the interface. The case is
/// part of the style rather than of the name, which is why it happens here and
/// is not stored.
fn subtitle(summary: &TrackSummary) -> String {
    let line = match (summary.artist.as_deref(), summary.album.as_deref()) {
        (Some(artist), Some(album)) => format!("{artist} · {album}"),
        (Some(artist), None) => artist.to_owned(),
        (None, Some(album)) => album.to_owned(),
        (None, None) => String::new(),
    };
    line.to_uppercase()
}

#[cfg(test)]
mod tests {
    use cadenza_core::application::view_state::PlayerView;
    use cadenza_core::domain::ids::MediaFileId;
    use cadenza_core::domain::playback::PlaybackState;
    use cadenza_core::domain::track::TrackSummary;
    use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition};

    use super::fields;

    #[test]
    fn an_empty_player_shows_nothing_rather_than_placeholders() {
        let shown = fields(&PlayerView::default());

        assert_eq!(shown.title, "");
        assert_eq!(shown.subtitle, "");
        assert_eq!(shown.initial, "");
        assert_eq!(shown.position, "00:00");
        assert!(!shown.loaded);
        assert!(!shown.playing);
    }

    #[test]
    fn a_loaded_track_reports_its_own_position_and_length() {
        let media_file_id = MediaFileId::new();
        let view = PlayerView {
            state: PlaybackState::Playing,
            track: Some(TrackSummary {
                media_file_id,
                title: "Mysterons".to_owned(),
                artist: Some("Portishead".to_owned()),
                album: None,
                duration: DurationMs::from_secs(300),
            }),
            position: PlaybackPosition::from_secs(75),
            duration: DurationMs::from_secs(300),
            ..PlayerView::default()
        };

        let shown = fields(&view);
        assert_eq!(shown.title, "Mysterons");
        assert_eq!(shown.subtitle, "PORTISHEAD", "no album, so no separator");
        assert_eq!(shown.initial, "M");
        assert_eq!(shown.position, "01:15");
        assert_eq!(shown.duration, "05:00");
        assert_eq!(shown.playing_id, media_file_id.to_string());
        assert!((shown.progress - 0.25).abs() < f32::EPSILON);
        assert!(shown.playing && shown.loaded);
    }

    #[test]
    fn artist_and_album_are_joined_only_when_both_are_there() {
        let base = TrackSummary {
            media_file_id: MediaFileId::new(),
            title: "\"Heroes\"".to_owned(),
            artist: Some("David Bowie".to_owned()),
            album: Some("\"Heroes\"".to_owned()),
            duration: DurationMs::from_secs(371),
        };

        let both = fields(&PlayerView {
            track: Some(base.clone()),
            ..PlayerView::default()
        });
        assert_eq!(both.subtitle, "DAVID BOWIE · \"HEROES\"");
        assert_eq!(both.initial, "H", "the quote is not the initial");

        let no_album = fields(&PlayerView {
            track: Some(TrackSummary {
                album: None,
                ..base.clone()
            }),
            ..PlayerView::default()
        });
        assert_eq!(no_album.subtitle, "DAVID BOWIE");

        let neither = fields(&PlayerView {
            track: Some(TrackSummary {
                artist: None,
                album: None,
                ..base
            }),
            ..PlayerView::default()
        });
        assert_eq!(neither.subtitle, "", "a dangling separator reads as a bug");
    }
}
