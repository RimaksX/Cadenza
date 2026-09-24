//! What plays next.

use super::*;

impl Controller {
    /// What plays on, named: the list, the artist or the station it comes
    /// from, rather than only the kind of thing it is. The kind alone where
    /// the name cannot be had.
    fn source_label(&self, from: Continuation) -> String {
        let named = match self.services.queue.playing_from() {
            Some(QueueOrigin::Playlist(id)) => self.services.playlists.get(id).ok().map(|list| {
                let name = crate::text::tr_owned(list.name.as_str());
                crate::text::tr1("NEXT FROM “{}”", &name.to_uppercase())
            }),
            Some(QueueOrigin::Artist(id)) => {
                self.services.library.artist(id).ok().map(|artist| {
                    crate::text::tr1("NEXT BY {}", &artist.name.as_str().to_uppercase())
                })
            }
            Some(QueueOrigin::Radio(_)) => self.services.radio.mood().map(|mood| {
                let name = crate::text::tr_owned(&mood.name);
                crate::text::tr1("NEXT ON “{}”", &name.to_uppercase())
            }),
            Some(QueueOrigin::Library) | None => None,
        };
        named.unwrap_or_else(|| {
            match from {
                Continuation::Library => crate::text::tr("NEXT FROM THE LIBRARY"),
                Continuation::Playlist => crate::text::tr("NEXT FROM THE PLAYLIST"),
                Continuation::Artist => crate::text::tr("NEXT BY THE ARTIST"),
                Continuation::Station => crate::text::tr("NEXT ON THE STATION"),
            }
            .to_owned()
        })
    }

    /// Opens the page of what is playing's source, and says which page that
    /// is, for the shell to turn to.
    pub fn open_source(&self) -> &'static str {
        match self.services.queue.playing_from() {
            Some(QueueOrigin::Playlist(id)) => {
                self.open_playlist(&id.to_string());
                "playlist"
            }
            Some(QueueOrigin::Artist(id)) => {
                self.open_artist(&id.to_string());
                "artist"
            }
            Some(QueueOrigin::Radio(_)) => "radio",
            Some(QueueOrigin::Library) | None => "library",
        }
    }

    /// Empties the queue, leaving what is playing where it is.
    pub fn clear_queue(&self) {
        self.run(|| self.services.queue.clear());
        self.refresh_queue();
    }

    /// Re-reads what is waiting to play.
    ///
    /// Not on the tick: it reads the whole library to resolve titles, and the
    /// queue only changes when something is asked of it.
    pub fn refresh_queue(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let queue_ui = window.global::<Queue>();

        let waiting = match self.services.queue.upcoming() {
            Ok(waiting) => waiting,
            // The same first-run state the library shows; the empty view says
            // what to do about it.
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        queue_ui.set_tracks(ModelRc::new(TrackRows::new(
            Arc::clone(&self.services.library),
            Rc::clone(&self.covers),
            &waiting,
        )));

        // And what plays on after it, which is most of what "next" means while
        // nobody has queued anything.
        let (label, then) = match self.services.queue.continuation(QUEUE_THEN) {
            Ok(Some((from, rows))) => (self.source_label(from), rows),
            Ok(None) | Err(CoreError::NoActiveProfile) => (String::new(), Vec::new()),
            Err(err) => {
                self.report(&err);
                (String::new(), Vec::new())
            }
        };
        queue_ui.set_then_label(label.into());
        queue_ui.set_then(ModelRc::new(TrackRows::new(
            Arc::clone(&self.services.library),
            Rc::clone(&self.covers),
            &then,
        )));
    }

    /// Jumps to a track in what plays on after the queue.
    pub fn play_following(&self, id: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services.queue.play_following(media_file_id)
        });
        self.refresh_queue();
    }

    /// Puts a track in the manual queue, at the end or with `next` straight
    /// after the current track.
    ///
    /// Nothing starts playing: the point of the manual queue is that it plays
    /// after what is on now.
    pub fn enqueue(&self, id: &str, next: bool) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            if next {
                self.services.queue.play_next(media_file_id)
            } else {
                self.services.queue.enqueue(media_file_id)
            }
        });
        self.refresh_queue();
    }

    /// Jumps to a waiting track, by its place in the queue's own listing.
    ///
    /// Not [`Self::play`]: that starts a track *and builds a queue behind it*
    /// from the library, which is the last thing a click inside the queue
    /// should do.
    pub fn play_queued(&self, position: i32) {
        let Ok(position) = usize::try_from(position) else {
            return;
        };
        self.run(|| self.services.queue.play_at(position));
        self.refresh_queue();
    }

    /// Takes a track out of the queue, by its place in the queue's own listing.
    pub fn remove_from_queue(&self, position: i32) {
        let Ok(position) = usize::try_from(position) else {
            return;
        };
        self.run(|| self.services.queue.remove_at(position));
        self.refresh_queue();
    }

    /// Moves a waiting track to another row of the queue, as it was carried.
    pub fn reorder_queue(&self, from: i32, to: i32) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        self.run(|| self.services.queue.move_queued(from, to));
        self.refresh_queue();
    }
}
