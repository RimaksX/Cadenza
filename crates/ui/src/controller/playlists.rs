//! The playlists and the one open.

use super::*;

impl Controller {
    /// Re-reads the index of playlists.
    pub fn refresh_playlists(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let playlists_ui = window.global::<Playlists>();

        let summaries = match self.services.playlists.list() {
            Ok(summaries) => summaries,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        playlists_ui.set_hint(crate::text::tr(NO_PLAYLISTS_HINT).into());
        // Without the automatic one: the line counts what this page holds, and
        // the favourites are pinned on Home instead.
        let own: Vec<_> = summaries
            .iter()
            .filter(|summary| !summary.playlist.is_favourites())
            .cloned()
            .collect();
        playlists_ui.set_summary(playlist_vm::summary_line(&own).into());

        // The favourites list leaves the grid's model for Home's banner.
        let cards = playlist_vm::cards(&summaries, |path| self.picture(path));
        let (banner, rest): (Vec<_>, Vec<_>) = cards.into_iter().partition(|card| card.automatic);

        playlists_ui.set_has_favourites(!banner.is_empty());
        if let Some(card) = banner.into_iter().next() {
            let chosen = summaries
                .iter()
                .find(|summary| summary.playlist.is_favourites())
                .and_then(|summary| summary.cover.as_deref())
                .filter(|_| card.has_cover);
            let tint = match chosen {
                Some(path) => self.tint_for(path),
                None => favourites_photo_tint(),
            };
            playlists_ui.set_favourites_tinted(tint.is_some());
            if let Some(tint) = tint {
                playlists_ui.set_favourites_tint(tint);
            }
            playlists_ui.set_favourites(card);
        }
        playlists_ui.set_playlists(ModelRc::new(VecModel::from(rest)));
        playlists_ui.set_open_options(ModelRc::new(VecModel::from(playlist_vm::options(
            &summaries,
        ))));
    }

    /// Opens one playlist's page.
    pub fn open_playlist(&self, id: &str) {
        let playlist_id = match PlaylistId::parse(id) {
            Ok(playlist_id) => playlist_id,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        // A pick in one list is not a pick in the next.
        if *self.open_playlist.borrow() != Some(playlist_id) {
            self.playlist_picking.clear();
        }
        *self.open_playlist.borrow_mut() = Some(playlist_id);
        self.shown.set(Section::Playlist);
        if let Some(window) = self.window.upgrade() {
            window.global::<Playlists>().set_open_id(id.into());
        }
        self.refresh_open_playlist();
    }

    /// Makes a playlist and shows it in the index.
    pub fn create_playlist(&self, name: &str) {
        self.run(|| self.services.playlists.create(name).map(|_| ()));
        self.refresh_playlists();
    }

    /// Starts a playlist at its first track, without opening its page.
    pub fn play_playlist(&self, id: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            let tracks = self.services.playlists.tracks_of(playlist_id)?;
            let first = tracks
                .first()
                .ok_or_else(|| CoreError::invalid("playlist", "has nothing to play"))?;

            self.services
                .queue
                .play_playlist(playlist_id, first.media_file_id)
        });
        self.refresh_queue();
    }

    /// Plays a playlist in no order: shuffle on, from a track chosen by
    /// chance.
    pub fn shuffle_playlist(&self, id: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            let tracks = self.services.playlists.tracks_of(playlist_id)?;
            let from = chance(&tracks)
                .ok_or_else(|| CoreError::invalid("playlist", "has nothing to play"))?;
            self.services
                .queue
                .play_playlist(playlist_id, from.media_file_id)?;
            // After the start rather than before it: a list that cannot be
            // started leaves the queue's shuffle as it was.
            if !self.services.queue.view().shuffle {
                self.services.queue.toggle_shuffle()?;
            }
            Ok(())
        });
        self.refresh_queue();
        self.refresh_player();
    }

    /// Renames one.
    pub fn rename_playlist(&self, id: &str, name: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            self.services
                .playlists
                .rename(playlist_id, name)
                .map(|_| ())
        });
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Deletes one. Its tracks stay in the library.
    pub fn delete_playlist(&self, id: &str) {
        self.run(|| {
            let playlist_id = PlaylistId::parse(id)?;
            self.services.playlists.delete(playlist_id)
        });

        // The page for a deleted playlist has nothing to show, so the index is
        // where the listener goes back to — and the markup has already taken
        // them there, because the tile they used is on it.
        if *self.open_playlist.borrow() == PlaylistId::parse(id).ok() {
            *self.open_playlist.borrow_mut() = None;
        }
        self.refresh_playlists();
    }

    /// Puts a track at the end of a playlist.
    ///
    /// `track` may name the picked rows of a list instead: see [`Self::tracks_named`].
    pub fn add_to_playlist(&self, track: &str, playlist: &str) {
        let tracks = self.tracks_named(track);
        self.run(|| {
            let playlist_id = PlaylistId::parse(playlist)?;
            for media_file_id in &tracks {
                self.services
                    .playlists
                    .add_track(playlist_id, *media_file_id)?;
            }
            Ok(())
        });
        self.unpick_named(track);
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Makes a playlist and puts a track in it, which is one act rather than
    /// two: nobody names a list and then wonders why it is empty.
    pub fn create_playlist_with(&self, track: &str, name: &str) {
        let tracks = self.tracks_named(track);
        self.run(|| {
            let playlist = self.services.playlists.create(name)?;
            for media_file_id in &tracks {
                self.services
                    .playlists
                    .add_track(playlist.id, *media_file_id)?;
            }
            Ok(())
        });
        self.unpick_named(track);
        self.refresh_playlists();
    }

    /// Takes the entry at a position out of the open playlist.
    pub fn remove_from_playlist(&self, position: i32) {
        let (Some(playlist_id), Ok(position)) =
            (*self.open_playlist.borrow(), usize::try_from(position))
        else {
            return;
        };

        self.run(|| self.services.playlists.remove_at(playlist_id, position));
        self.refresh_playlists();
        self.refresh_open_playlist();
    }

    /// Re-reads whatever playlist page is open.
    pub(super) fn refresh_open_playlist(&self) {
        let (Some(window), Some(playlist_id)) =
            (self.window.upgrade(), *self.open_playlist.borrow())
        else {
            return;
        };
        let playlists_ui = window.global::<Playlists>();

        let playlist = match self.services.playlists.get(playlist_id) {
            Ok(playlist) => playlist,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let tracks = match self.services.playlists.tracks_of(playlist_id) {
            Ok(tracks) => tracks,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        // The page's head wears the list's cover, and its colour. The
        // favourites without a picture of their own stand on the photograph
        // Home shows them on.
        let path = self.services.playlists.cover_of(playlist_id).ok().flatten();
        let cover = path
            .as_deref()
            .map(|path| self.picture(path))
            .unwrap_or_default();
        let has_cover = cover.size().width > 0;
        let tint = match path.as_deref() {
            Some(path) if has_cover => self.tint_for(path),
            _ if playlist.is_favourites() => favourites_photo_tint(),
            _ => None,
        };
        playlists_ui.set_open_has_cover(has_cover);
        playlists_ui.set_open_cover(cover);
        playlists_ui.set_open_tinted(tint.is_some());
        if let Some(tint) = tint {
            playlists_ui.set_open_tint(tint);
        }
        playlists_ui.set_open_name(playlist.name.as_str().into());
        playlists_ui.set_open_automatic(playlist.is_favourites());
        playlists_ui.set_open_hint(crate::text::tr(EMPTY_PLAYLIST_HINT).into());
        playlists_ui.set_open_summary(library_vm::summary_line(&tracks).into());
        playlists_ui.set_open_tracks(self.playlist_picking.model(TrackRows::new(
            Arc::clone(&self.services.library),
            Rc::clone(&self.covers),
            &tracks,
        )));
        playlists_ui.set_open_picked(count(&self.playlist_picking));
    }

    /// Starts a track from the open playlist, with the rest of the list behind
    /// it.
    pub fn play_from_playlist(&self, id: &str) {
        let Some(playlist_id) = *self.open_playlist.borrow() else {
            return;
        };

        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services
                .queue
                .play_playlist(playlist_id, media_file_id)
        });
        self.refresh_queue();
        self.refresh_radio();
    }

    /// The same two, for a list.
    pub fn choose_playlist_cover(&self, id: &str) {
        let Ok(playlist_id) = PlaylistId::parse(id) else {
            return;
        };
        self.run(|| self.services.playlists.choose_cover(playlist_id).map(drop));
        self.forget_pictures();
        self.refresh_playlists();
    }

    pub fn clear_playlist_cover(&self, id: &str) {
        let Ok(playlist_id) = PlaylistId::parse(id) else {
            return;
        };
        self.run(|| self.services.playlists.clear_cover(playlist_id));
        self.forget_pictures();
        self.refresh_playlists();
    }

    /// Writes a playlist out as a file another player can read.
    pub fn export_playlist(&self, id: &str) {
        let Ok(playlist_id) = PlaylistId::parse(id) else {
            return;
        };
        match self.services.library.export_playlist(playlist_id) {
            Ok(Some(path)) => self.say(&crate::text::tr1(
                "Saved as {}",
                &path
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            )),
            Ok(None) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Reads a playlist in from a file another player wrote.
    pub fn import_playlist(&self) {
        match self.services.library.import_playlist() {
            Ok(Some(import)) => {
                self.say(
                    &crate::text::tr("“{}” holds {} of the {} tracks the file lists")
                        .replacen("{}", &import.name, 1)
                        .replacen("{}", &import.joined.to_string(), 1)
                        .replacen("{}", &import.listed.to_string(), 1),
                );
                self.refresh_playlists();
                self.after_library_change();
            }
            Ok(None) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Moves a row of the open playlist to another row, as it was carried.
    pub fn move_in_playlist(&self, from: i32, to: i32) {
        let (Some(playlist_id), Ok(from), Ok(to)) = (
            *self.open_playlist.borrow(),
            usize::try_from(from),
            usize::try_from(to),
        ) else {
            return;
        };
        self.run(|| self.services.playlists.move_entry(playlist_id, from, to));
        self.refresh_open_playlist();
        self.refresh_playlists();
    }
}
