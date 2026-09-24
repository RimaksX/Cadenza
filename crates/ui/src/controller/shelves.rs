//! The library by artist: the shelf, and one artist open.

use cadenza_core::application::services::ArtistSummary;
use cadenza_core::domain::ids::ArtistId;

use super::*;

impl Controller {
    /// Re-reads the artists.
    pub fn refresh_artists(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let artists = match self.services.library.artists() {
            Ok(artists) => artists,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };
        let cards: Vec<PlaylistCardData> = artists
            .iter()
            .map(|artist| PlaylistCardData {
                id: artist.id.to_string().into(),
                name: artist.name.as_str().into(),
                meta: artist_tile_line(artist).into(),
                ..PlaylistCardData::default()
            })
            .collect();
        let shelves = window.global::<Shelves>();
        shelves.set_artists_summary(
            counted(artists.len(), "artist|one", "artists|few", "artists|many").into(),
        );
        shelves.set_artists(ModelRc::new(VecModel::from(cards)));
        self.fill_tiles(
            artists
                .iter()
                .map(|artist| {
                    let tracks = artist.tracks.iter().map(|track| track.media_file_id);
                    (artist.id.to_string(), tracks.collect())
                })
                .collect(),
        );
    }

    /// Puts a picture on each tile of the shelf: the first cover among the
    /// artist's tracks, in the size a tile draws.
    ///
    /// On a thread of its own, because the tile-sized copy of a cover is made
    /// the first time it is asked for, and a shelf of a few hundred artists
    /// opened for the first time is a few hundred pictures to shrink. The
    /// tiles are drawn at once with their stand-ins and take their pictures
    /// as they come.
    fn fill_tiles(&self, cards: Vec<(String, Vec<MediaFileId>)>) {
        let library = Arc::clone(&self.services.library);
        let window = self.window.clone();
        std::thread::spawn(move || {
            for (card, tracks) in cards {
                let Some(path) = tracks
                    .iter()
                    .find_map(|track| library.tile_for(*track).ok().flatten())
                else {
                    continue;
                };
                let posted = window.upgrade_in_event_loop(move |window| {
                    let model = window.global::<Shelves>().get_artists();
                    // By identifier, not by place: the shelf may have been
                    // drawn again since this thread was started.
                    let Some(row) = (0..model.row_count())
                        .find(|&row| model.row_data(row).is_some_and(|data| data.id == card))
                    else {
                        return;
                    };
                    if let (Some(mut data), Ok(cover)) =
                        (model.row_data(row), slint::Image::load_from_path(&path))
                    {
                        data.cover = cover;
                        data.has_cover = true;
                        model.set_row_data(row, data);
                    }
                });
                if posted.is_err() {
                    return;
                }
            }
        });
    }

    /// Opens the page of whoever made what is playing, and says whether there
    /// was one: a file with no artist tag has no page to go to.
    pub fn show_playing_artist(&self) -> bool {
        let Some(playing) = self
            .services
            .playback
            .view()
            .track
            .map(|track| track.media_file_id)
        else {
            return false;
        };
        let found = self.services.library.artists().ok().and_then(|artists| {
            artists.into_iter().find(|artist| {
                artist
                    .tracks
                    .iter()
                    .any(|track| track.media_file_id == playing)
            })
        });
        match found {
            Some(artist) => {
                self.open_artist(&artist.id.to_string());
                true
            }
            None => false,
        }
    }

    /// Opens an artist on their own page.
    pub fn open_artist(&self, id: &str) {
        match ArtistId::parse(id) {
            Ok(artist_id) => {
                self.open_artist.set(Some(artist_id));
                self.shown.set(Section::Artist);
                self.refresh_open_artist();
            }
            Err(err) => self.report(&err),
        }
    }

    /// Re-reads the artist that is open.
    pub(super) fn refresh_open_artist(&self) {
        let (Some(window), Some(id)) = (self.window.upgrade(), self.open_artist.get()) else {
            return;
        };
        let (name, summary, tracks) = match self.services.library.artist(id) {
            Ok(artist) => (artist.name.clone(), artist_line(&artist), artist.tracks),
            // Their last track was edited away, or taken out: the page stays,
            // empty, under the name it had, rather than failing.
            Err(CoreError::NotFound { .. }) => (
                window.global::<Shelves>().get_open_name().to_string(),
                library_vm::summary_line(&[]),
                Vec::new(),
            ),
            Err(err) => {
                self.report(&err);
                return;
            }
        };
        // The page's head wears the first cover among their tracks, as their
        // tile does, and its colour. At the tile's size: the head draws it at
        // 176, and a full cover decoded here was kept for as long as the
        // window was open, one more for every artist visited.
        let cover = tracks
            .iter()
            .find_map(|track| {
                self.services
                    .library
                    .tile_for(track.media_file_id)
                    .ok()
                    .flatten()
            })
            .and_then(|path| slint::Image::load_from_path(&path).ok())
            .unwrap_or_default();
        let tint = tint_of(&cover);
        let shelves = window.global::<Shelves>();
        shelves.set_open_has_cover(cover.size().width > 0);
        shelves.set_open_cover(cover);
        shelves.set_open_tinted(tint.is_some());
        if let Some(tint) = tint {
            shelves.set_open_tint(tint);
        }
        shelves.set_open_name(name.into());
        shelves.set_open_summary(summary.into());
        shelves.set_open_tracks(ModelRc::new(TrackRows::new(
            Arc::clone(&self.services.library),
            Rc::clone(&self.covers),
            &tracks,
        )));
    }

    /// Redraws the shelf or the open artist, if either is on screen: an edit
    /// to a track's artist moves it from one to another, and the page being
    /// looked at should show that at once.
    pub(super) fn refresh_shown_shelf(&self) {
        match self.shown.get() {
            Section::Artists => self.refresh_artists(),
            Section::Artist => self.refresh_open_artist(),
            _ => {}
        }
    }

    /// Plays everything by an artist from the first of it.
    pub fn play_artist(&self, id: &str) {
        if let Ok(artist_id) = ArtistId::parse(id) {
            self.play_artist_from(artist_id, None);
        }
    }

    /// Plays the open artist from one of their tracks, or from the start.
    pub fn play_open(&self, track: Option<&str>) {
        if let Some(artist_id) = self.open_artist.get() {
            self.play_artist_from(artist_id, track);
        }
    }

    /// Plays everything by an artist shuffled: shuffle on, from a track
    /// chosen by chance.
    pub fn shuffle_artist(&self, id: &str) {
        if let Ok(artist_id) = ArtistId::parse(id) {
            self.shuffle_artist_id(artist_id);
        }
    }

    /// Plays the open artist shuffled.
    pub fn shuffle_open(&self) {
        if let Some(artist_id) = self.open_artist.get() {
            self.shuffle_artist_id(artist_id);
        }
    }

    fn shuffle_artist_id(&self, artist_id: ArtistId) {
        self.run(|| {
            let tracks = self.services.library.artist(artist_id)?.tracks;
            let from = chance(&tracks)
                .ok_or_else(|| CoreError::invalid("artist", "there is nothing to play"))?;
            self.services
                .queue
                .play_artist(artist_id, from.media_file_id)?;
            // After the start rather than before it: an artist who cannot be
            // started leaves the queue's shuffle as it was.
            if !self.services.queue.view().shuffle {
                self.services.queue.toggle_shuffle()?;
            }
            Ok(())
        });
        self.refresh_queue();
        self.refresh_radio();
    }

    fn play_artist_from(&self, artist_id: ArtistId, track: Option<&str>) {
        self.run(|| {
            let from = match track {
                Some(track) => MediaFileId::parse(track)?,
                None => self
                    .services
                    .library
                    .artist(artist_id)?
                    .tracks
                    .first()
                    .map(|track| track.media_file_id)
                    .ok_or_else(|| CoreError::invalid("artist", "there is nothing to play"))?,
            };
            self.services.queue.play_artist(artist_id, from)
        });
        self.refresh_queue();
        self.refresh_radio();
    }
}

/// `3 artists`, in the listener's language.
fn counted(count: usize, one: &'static str, few: &'static str, many: &'static str) -> String {
    format!(
        "{count} {}",
        crate::text::plural(count as u64, one, few, many)
    )
}

/// On an artist's page: how many albums, and how much.
fn artist_line(artist: &ArtistSummary) -> String {
    let mut parts = Vec::new();
    if artist.albums > 0 {
        parts.push(counted(
            artist.albums,
            "album|one",
            "albums|few",
            "albums|many",
        ));
    }
    parts.push(library_vm::summary_line(&artist.tracks));
    parts.join(" · ")
}

/// Under an artist's tile: how many albums and tracks, without the time.
fn artist_tile_line(artist: &ArtistSummary) -> String {
    let tracks = counted(
        artist.tracks.len(),
        "track|one",
        "tracks|few",
        "tracks|many",
    );
    if artist.albums > 0 {
        format!(
            "{} · {tracks}",
            counted(artist.albums, "album|one", "albums|few", "albums|many")
        )
    } else {
        tracks
    }
}
