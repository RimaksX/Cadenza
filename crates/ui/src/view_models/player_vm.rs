//! Turning the player's state into what the bar shows.

use cadenza_core::application::view_state::PlayerView;

/// What the player bar draws, as strings and numbers the markup can bind.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerFields {
    /// Title of the loaded track.
    pub title: String,
    /// Artist of the loaded track.
    pub artist: String,
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
        artist: track
            .and_then(|summary| summary.artist.clone())
            .unwrap_or_default(),
        position: format!("{}", view.position.elapsed()),
        duration: format!("{}", view.duration),
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
        assert_eq!(shown.artist, "");
        assert_eq!(shown.position, "0:00");
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
        assert_eq!(shown.artist, "Portishead");
        assert_eq!(shown.position, "1:15");
        assert_eq!(shown.duration, "5:00");
        assert_eq!(shown.playing_id, media_file_id.to_string());
        assert!((shown.progress - 0.25).abs() < f32::EPSILON);
        assert!(shown.playing && shown.loaded);
    }
}
