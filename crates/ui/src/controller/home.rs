//! The home page, and the favourites it leads with.

use super::*;

impl Controller {
    /// What Home lists: the tracks heard lately, the newest in the library,
    /// and a handful of what has gone quiet.
    pub(super) fn refresh_home(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let home_ui = window.global::<Home>();
        let lists = (|| {
            let recent = self.services.stats.recently_played(HOME_RECENT)?;
            let library = self.services.library.summaries()?;
            home_ui.set_track_count(i32::try_from(library.len()).unwrap_or(i32::MAX));
            let fresh: Vec<_> = library.into_iter().take(HOME_TILES).collect();
            // Full covers rather than thumbnails: a tile is drawn larger than a
            // thumbnail is, and there are six of them rather than thousands.
            // A different handful each time Home is drawn: the list is the
            // whole of what has gone quiet, and five from the same end of it
            // would be the same five every day.
            let mut quiet = self.services.stats.not_heard_lately()?;
            if let Some(start) = pick(quiet.len()) {
                quiet.rotate_left(start);
            }
            quiet.truncate(HOME_RECENT);
            Ok::<_, CoreError>((recent, fresh, quiet))
        })();
        // What "any playlist" can land on: the listener's own lists with
        // something in them. Not the favourites, which Home plays from its
        // own band, and which the playlists page does not count either.
        let playable = self
            .services
            .playlists
            .list()
            .map(|all| all.iter().filter(|list| is_chance_list(list)).count())
            .unwrap_or(0);
        home_ui.set_playlist_count(i32::try_from(playable).unwrap_or(i32::MAX));
        let (recent, fresh, quiet) = match lists {
            Ok(lists) => lists,
            Err(CoreError::NoActiveProfile) => Default::default(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };
        let rows = |tracks: &[TrackSummary]| {
            ModelRc::new(TrackRows::new(
                Arc::clone(&self.services.library),
                Rc::clone(&self.covers),
                tracks,
            ))
        };
        home_ui.set_recent(rows(&recent));
        let mut tiles = library_vm::rows(&fresh);
        for (tile, summary) in tiles.iter_mut().zip(&fresh) {
            if let Ok(Some(path)) = self.services.library.cover_for(summary.media_file_id) {
                tile.cover = self.picture(&path);
            }
        }
        home_ui.set_fresh(ModelRc::new(VecModel::from(tiles)));
        home_ui.set_forgotten(rows(&quiet));
    }

    /// Plays a track from the library chosen by chance, the library behind it.
    pub fn play_random_track(&self) {
        let library = match self.services.library.summaries() {
            Ok(library) => library,
            Err(err) => return self.report(&err),
        };
        match chance(&library) {
            Some(track) => self.play(&track.media_file_id.to_string()),
            None => self.say(crate::text::tr(
                "the library is empty - add a folder in Settings",
            )),
        }
    }

    /// Plays a playlist chosen by chance, from among those with something in.
    pub fn play_random_playlist(&self) {
        let playlists = match self.services.playlists.list() {
            Ok(playlists) => playlists,
            Err(err) => return self.report(&err),
        };
        let playable: Vec<_> = playlists.into_iter().filter(is_chance_list).collect();
        match chance(&playable) {
            Some(summary) => self.play_playlist(&summary.playlist.id.to_string()),
            None => self.say(crate::text::tr("no playlist has anything in it yet")),
        }
    }

    /// Empties the favourites, after the dialog asked.
    pub fn clear_favourites(&self) {
        self.run(|| self.services.playlists.clear_favourites());
        self.refresh_playlists();
        self.refresh_player();
    }

    /// Rebuilds the played part of the favourites list.
    ///
    /// A first run with nobody listening yet has nothing to rebuild, which is
    /// a state rather than a failure. Anything else is reported the way every
    /// other failure here is - the list is rebuilt from the counts whenever a
    /// track ends, so a listener who sees this once and not again has already
    /// had it put right.
    pub(super) fn refresh_favourites(&self) {
        self.favourite.take();
        match self.services.playlists.refresh_favourites() {
            Ok(()) | Err(CoreError::NoActiveProfile) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Re-reads the month of listening.
    pub fn refresh_listening(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let listening_ui = window.global::<Listening>();

        let report = match self.services.stats.report() {
            Ok(report) => report,
            Err(CoreError::NoActiveProfile) => return,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        listening_ui.set_kept(report.keeping);
        listening_ui.set_summary(stats_vm::summary_line(&report).into());
        listening_ui.set_heard(stats_vm::heard(report.summary.listened).into());
        listening_ui.set_listens(report.summary.completed.to_string().into());
        listening_ui.set_tracks(report.summary.tracks.to_string().into());
        listening_ui.set_skip_rate(stats_vm::skip_rate(&report).into());

        let rows: Vec<TopTrackData> = report
            .top
            .iter()
            .enumerate()
            .map(|(index, (track, count))| TopTrackData {
                id: track.media_file_id.to_string().into(),
                rank: format!("{}", index + 1).into(),
                title: track.title.as_str().into(),
                subtitle: track.artist.clone().unwrap_or_default().into(),
                plays: stats_vm::plays(*count).into(),
                cover: self.thumbnail(track.media_file_id),
            })
            .collect();
        listening_ui.set_top_tracks(ModelRc::new(VecModel::from(rows)));
    }
}

/// A list "any playlist" may land on: one of the listener's own, with
/// something in it.
fn is_chance_list(summary: &cadenza_core::application::services::PlaylistSummary) -> bool {
    summary.track_count > 0 && !summary.playlist.is_favourites()
}
