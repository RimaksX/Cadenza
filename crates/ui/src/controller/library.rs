//! The library page: reading it, ordering and searching it, the tracks in it and the decisions about them.

use super::*;

impl Controller {
    /// Re-reads the library. Cheap enough to call on any change that could have
    /// touched it, and the only thing that reads the whole table.
    pub fn refresh_library(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let library_ui = window.global::<Library>();

        // The orders on offer, and the one in force. Built here, after the language is chosen, rather than in
        // the markup: what a library can be sorted by is a decision, and the
        // markup is where decisions are drawn rather than made.
        library_ui.set_order_label(self.order.get().label().into());
        library_ui.set_orders(ModelRc::new(VecModel::from(
            Order::all()
                .into_iter()
                .map(|order| MenuItemData {
                    action: order.name().into(),
                    label: order.choice().into(),
                    meta: SharedString::new(),
                    destructive: false,
                })
                .collect::<Vec<_>>(),
        )));

        match self.services.library.summaries() {
            // Straight into the copy the search will filter, rather than into
            // a local that is then cloned into it: five thousand rows is not a
            // thing to hold twice for the sake of a shorter line.
            Ok(summaries) => *self.shown_library.borrow_mut() = summaries,
            // Not an error worth reporting: it is the first-run state, and the
            // window is showing the welcome screen rather than this page. A
            // hint used to be set here telling somebody to open Settings and
            // add a listener, which is the thing that screen now does - a
            // sentence nobody could reach and which had stopped being true.
            Err(CoreError::NoActiveProfile) => {
                self.shown_library.borrow_mut().clear();
                self.shown_order.borrow_mut().clear();
                library_ui.set_tracks(ModelRc::new(VecModel::from(Vec::new())));
                library_ui.set_summary("no profile".into());
                return;
            }
            Err(err) => {
                self.report(&err);
                return;
            }
        }

        let query = self.query.borrow().clone();
        self.show_library(&window, &query);
        self.refresh_shown_shelf();
    }

    /// Which row of the library as it is shown is the one playing, or -1.
    ///
    /// The library as *shown*: an order and a search both move it, and pointing
    /// at where a track would be in some other arrangement would scroll to the
    /// wrong row.
    pub(super) fn playing_row(&self, playing_id: &str) -> i32 {
        let Ok(playing) = MediaFileId::parse(playing_id) else {
            return -1;
        };
        self.shown_order
            .borrow()
            .iter()
            .position(|id| *id == playing)
            .and_then(|at| i32::try_from(at).ok())
            .unwrap_or(-1)
    }

    /// Draws the library that was last read, filtered by whatever is typed.
    ///
    /// Both callers pass the query rather than reading it, because one of them
    /// is in the middle of writing it.
    pub(super) fn show_library(&self, window: &AppWindow, query: &str) {
        let library_ui = window.global::<Library>();
        let summaries = self.shown_library.borrow();
        let mut shown = library_vm::matching(&summaries, query);
        library_vm::arrange(&mut shown, self.order.get());
        *self.shown_order.borrow_mut() = shown.iter().map(|row| row.media_file_id).collect();

        library_ui.set_summary(library_vm::found_line(&shown, query, summaries.len()).into());
        library_ui.set_bare(summaries.is_empty());
        library_ui.set_empty_hint(if query.is_empty() {
            crate::text::tr(NO_TRACKS_HINT).into()
        } else {
            crate::text::tr(NO_MATCH_HINT).into()
        });
        library_ui.set_tracks(self.library_picking.model(TrackRows::new(
            Arc::clone(&self.services.library),
            Rc::clone(&self.covers),
            &shown,
        )));
        library_ui.set_picked(count(&self.library_picking));
    }

    /// Turns the library another way round.
    ///
    /// Re-sorts what was already read rather than reading it again, for the
    /// same reason searching does: an order is not a change to the library.
    pub fn set_order(&self, name: &str) {
        self.order.set(Order::from_name(name));

        if let Some(window) = self.window.upgrade() {
            window
                .global::<Library>()
                .set_order_label(self.order.get().label().into());
            let query = self.query.borrow().clone();
            self.show_library(&window, &query);
        }
    }

    /// Filters the library by what has been typed into its search field.
    ///
    /// In Rust rather than in the markup: what counts as a match is a decision,
    /// and decisions made here can be tested without a window.
    ///
    /// Filters what was already read rather than reading it again. A letter
    /// typed cannot have changed the library, and the read is the expensive
    /// half of the work.
    pub fn search(&self, query: &str) {
        *self.query.borrow_mut() = query.to_owned();

        if let Some(window) = self.window.upgrade() {
            self.show_library(&window, query);
        }
    }

    /// Takes a track out of this profile's library.
    ///
    /// A tombstone rather than a delete: the file stays on disk and every other
    /// profile keeps its own copy of the row.
    pub fn remove_from_library(&self, track: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(track)?;
            self.services.library.remove_track(media_file_id)
        });

        // It may have been in lists and in the queue, and both of those show
        // titles they can no longer resolve.
        self.refresh_library();
        self.refresh_playlists();
        self.refresh_open_playlist();
        self.refresh_queue();
    }

    /// Re-reads what is waiting for a decision.
    pub fn refresh_reviews(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let library_ui = window.global::<Library>();

        let cards = match self.services.library.review_cards() {
            Ok(cards) => cards,
            Err(CoreError::NoActiveProfile) => Vec::new(),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        let rows: Vec<ReviewRowData> = cards
            .iter()
            .map(|card| ReviewRowData {
                id: card.id.to_string().into(),
                path: card
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
                    .into(),
                reason: review_vm::reason(card.reason).into(),
                existing: review_vm::collides_with(card).into(),
                duplicate: card.existing.is_some(),
            })
            .collect();

        library_ui.set_reviews_summary(review_vm::summary_line(cards.len()).into());
        library_ui.set_reviews(ModelRc::new(VecModel::from(rows)));
    }

    /// Applies a decision and takes the row away.
    pub fn decide_review(&self, id: &str, choice: ReviewChoice) {
        self.run(|| {
            let review_id = ImportReviewId::parse(id)?;
            // Three named choices against three resolutions, and no wildcard.
            // The string form ended in `_ => AddAnyway`, so a misspelling
            // anywhere in the markup would have added a file the listener had
            // just asked to keep out, quietly and irreversibly.
            let resolution = match choice {
                ReviewChoice::Keep => ReviewResolution::KeepExisting,
                ReviewChoice::Replace => ReviewResolution::RemoveExisting,
                ReviewChoice::Add => ReviewResolution::AddAnyway,
            };
            self.services.library.resolve_review(review_id, resolution)
        });

        self.after_library_change();
    }

    /// Opens the editor on a track, or closes it when the id is empty.
    ///
    /// What it is called now is read here rather than taken from the row: a
    /// listing elides long titles, and the field would then offer the listener
    /// their own title with a dash in the middle of it.
    pub fn edit_track(&self, id: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let library_ui = window.global::<Library>();

        if id.is_empty() {
            library_ui.set_editing_id(String::new().into());
            return;
        }

        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            let track = self
                .services
                .library
                .summaries()?
                .into_iter()
                .find(|summary| summary.media_file_id == media_file_id)
                .ok_or_else(|| CoreError::not_found("track", media_file_id))?;

            library_ui.set_editing_title(track.title.as_str().into());
            library_ui.set_editing_artist(track.artist.clone().unwrap_or_default().into());
            library_ui.set_editing_album(track.album.clone().unwrap_or_default().into());
            library_ui.set_editing_id(id.into());
            Ok(())
        });
    }

    /// Writes a correction down and closes the editor.
    pub fn save_track(&self, id: &str, title: &str, artist: &str, album: &str) {
        self.run(|| {
            let media_file_id = MediaFileId::parse(id)?;
            self.services
                .library
                .edit_track(media_file_id, title, Some(artist), Some(album))
        });

        if let Some(window) = self.window.upgrade() {
            window
                .global::<Library>()
                .set_editing_id(String::new().into());
        }
        self.after_library_change();
    }

    /// Re-reads a screen the listener has just moved to.
    ///
    /// A page is drawn from a service at the moment something asks it to be,
    /// and between one visit and the next the service may have moved on
    /// without the page being told — a station ends because a track was
    /// played, a folder is scanned, a preset is renamed. Asking on arrival
    /// costs one query on a keypress and closes the whole class rather than
    /// the one case somebody happened to notice.
    /// Asks for a picture and puts it on a track.
    ///
    /// The chooser blocks the interface thread, which is what a modal dialog
    /// does. Nothing else may call it, and nothing else does.
    pub fn choose_cover(&self, id: &str) {
        let Ok(media_file_id) = MediaFileId::parse(id) else {
            return;
        };
        self.run(|| self.services.library.choose_cover(media_file_id).map(drop));
        self.refresh_cover(true);
    }

    /// Takes a chosen cover off, leaving whatever the file itself carries.
    pub fn clear_cover(&self, id: &str) {
        let Ok(media_file_id) = MediaFileId::parse(id) else {
            return;
        };
        self.run(|| self.services.library.clear_cover(media_file_id));
        self.refresh_cover(true);
    }

    /// Whether something from outside is being held over the window.
    ///
    /// Written only when it changes: this is asked twenty times a second, and
    /// every write to a property is a repaint of whatever reads it.
    pub fn carrying_files(&self, carrying: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if window.get_dropping() != carrying {
            window.set_dropping(carrying);
        }
    }

    /// Takes in what was let go of over the window.
    ///
    /// On a thread of its own. A folder dropped in can be five thousand files,
    /// and hashing them on the event loop would freeze the interface — the one
    /// that is meanwhile drawing a progress line for the music still playing.
    /// What comes back is a sentence in the player bar; the list itself
    /// refreshes the way any other change behind the listener's back does.
    pub fn accept_drop(&self, paths: Vec<PathBuf>) {
        let library = Arc::clone(&self.services.library);
        let window = self.window.clone();

        std::thread::spawn(move || {
            let (said, failed) = match library.accept_drop(&paths) {
                Ok(report) => (library_vm::taken_in(&report), false),
                Err(err) => (err.to_string(), true),
            };

            // Back on the event loop to say so: a window may only be touched
            // from the thread that runs it.
            let _ = window.upgrade_in_event_loop(move |window| {
                super::show_message(&window, &said, failed);
            });
        });
    }

    /// The library moved by the listener's own hand, so everything that lists
    /// it has to look again: what a change on disk moves, and the settings
    /// page, whose folders and counts are what the command was about. The
    /// queue is in the first half - a folder taken away takes its tracks out
    /// of it, which this used to leave on screen.
    pub(super) fn after_library_change(&self) {
        self.refresh_after_change();
        self.refresh_settings();
    }

    /// The button came up: nothing is carried any more, whatever the rows say.
    ///
    /// And when a carry was what it ended, the lists are told the pointer has
    /// gone. A list scrolls under a pressed button, and the button coming up
    /// at the end of a carry is heard by whatever took the carry, not by the
    /// list it began in - which went on scrolling with the pointer as if the
    /// button were still down. Leaving is the other thing that makes a list let
    /// go, and unlike a release it presses nothing: a row does not take it for
    /// a click and start playing. Only after a carry, so that an ordinary
    /// click does not blink the hover off whatever was clicked.
    pub fn carry_ended(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let transfer = window.global::<Transfer>();
        transfer.set_hint(SharedString::new());

        let carries = transfer.get_drags();
        if self.carries_seen.replace(carries) != carries {
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerExited);
        }
    }
}
