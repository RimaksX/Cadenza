//! Several rows picked at once, in the library or the open playlist, and one
//! thing done to all of them.

use super::*;

/// Which list a pick is in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickedIn {
    Library,
    Playlist,
}

impl PickedIn {
    /// How the markup names the picked rows of this list where one track's
    /// identifier would otherwise go - the playlist chooser is shared.
    const fn name(self) -> &'static str {
        match self {
            Self::Library => "picked:library",
            Self::Playlist => "picked:playlist",
        }
    }
}

impl Controller {
    fn picking(&self, list: PickedIn) -> &Picking {
        match list {
            PickedIn::Library => &self.library_picking,
            PickedIn::Playlist => &self.playlist_picking,
        }
    }

    /// Tells the markup how many are picked in `list`.
    fn show_picked(&self, list: PickedIn) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let picked = count(self.picking(list));
        match list {
            PickedIn::Library => window.global::<Library>().set_picked(picked),
            PickedIn::Playlist => window.global::<Playlists>().set_open_picked(picked),
        }
    }

    /// A row picked or let go, with the control key (`add`) or shift (`range`).
    pub fn pick(&self, list: PickedIn, row: i32, add: bool, range: bool) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        self.picking(list).pick(row, add, range);
        self.show_picked(list);
    }

    /// Lets every picked row of `list` go.
    pub fn unpick(&self, list: PickedIn) {
        self.picking(list).clear();
        self.show_picked(list);
    }

    /// The tracks `name` stands for: one track's identifier, or the picked
    /// rows of a list, in the order it shows them.
    pub(super) fn tracks_named(&self, name: &str) -> Vec<MediaFileId> {
        for list in [PickedIn::Library, PickedIn::Playlist] {
            if name == list.name() {
                return self
                    .picking(list)
                    .picked()
                    .into_iter()
                    .map(|(_, id)| id)
                    .collect();
            }
        }
        MediaFileId::parse(name).into_iter().collect()
    }

    /// Lets the rows go that `name` picked, once something was done with them.
    pub(super) fn unpick_named(&self, name: &str) {
        for list in [PickedIn::Library, PickedIn::Playlist] {
            if name == list.name() {
                self.unpick(list);
            }
        }
    }

    /// Puts every picked track at the end of the queue, in the order shown.
    pub fn queue_picked(&self, list: PickedIn) {
        let tracks = self.tracks_named(list.name());
        self.run(|| {
            for media_file_id in &tracks {
                self.services.queue.enqueue(*media_file_id)?;
            }
            Ok(())
        });
        self.say(&crate::text::tr1(
            "{} in the queue",
            &tracks.len().to_string(),
        ));
        self.unpick(list);
        self.refresh_queue();
    }

    /// Takes every picked row out: of the library, or of the open playlist.
    pub fn remove_picked(&self, list: PickedIn) {
        let picked = self.picking(list).picked();
        match list {
            PickedIn::Library => {
                self.run(|| {
                    for (_, media_file_id) in &picked {
                        self.services.library.remove_track(*media_file_id)?;
                    }
                    Ok(())
                });
                self.unpick(list);
                self.refresh_library();
                self.refresh_playlists();
                self.refresh_open_playlist();
                self.refresh_queue();
            }
            PickedIn::Playlist => {
                let Some(playlist_id) = *self.open_playlist.borrow() else {
                    return;
                };
                // From the bottom up, so each row is still where it was.
                self.run(|| {
                    for (row, _) in picked.iter().rev() {
                        self.services.playlists.remove_at(playlist_id, *row)?;
                    }
                    Ok(())
                });
                self.unpick(list);
                self.refresh_open_playlist();
                self.refresh_playlists();
            }
        }
    }
}
