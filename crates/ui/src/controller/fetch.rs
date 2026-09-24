//! Fetching from a link.

use super::*;

impl Controller {
    /// Says which of the two buttons the link in the box can answer.
    ///
    /// Read from the domain rather than decided here: which readings an address
    /// offers is a rule, and the interface's job is to draw the answer. Called
    /// on every change to the box, which costs four substring searches.
    pub fn link_typed(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let fetch_ui = window.global::<Fetch>();
        let readings = readings_of(&fetch_ui.get_link());
        fetch_ui.set_offers_a_track(readings.track);
        fetch_ui.set_offers_a_list(readings.list);
    }

    /// Brings in the one track a link points at.
    pub fn fetch_from_link(&self, link: &str) {
        self.fetch(link, FetchWhat::OneTrack);
    }

    /// Brings in every track of the playlist a link carries.
    ///
    /// A button of its own rather than a guess at the address, because the two
    /// readings of the same link are both reasonable and only the listener
    /// knows which they meant.
    pub fn fetch_playlist(&self, link: &str) {
        self.fetch(link, FetchWhat::WholePlaylist);
    }

    /// Asks the download to end. It stops at the next line the downloader
    /// prints, which is at most a second and usually less.
    ///
    /// What has already finished is kept, and yt-dlp's own record of it means
    /// pressing the button again carries on rather than starting over.
    pub fn stop_fetch(&self) {
        self.fetching.stopping.store(true, Ordering::Relaxed);
        if let Some(window) = self.window.upgrade() {
            window.global::<Fetch>().set_note("stopping…".into());
        }
    }

    /// Brings a track, or a playlist, in from a pasted link.
    ///
    /// The work happens on a thread, because it is a download and a conversion
    /// and the window has to keep drawing through both. What comes back comes
    /// back the way everything off the event loop does: written into shared
    /// state and read by the tick.
    pub(super) fn fetch(&self, link: &str, what: FetchWhat) {
        self.asked_for.set(what);

        // One at a time. Two downloads writing into one folder is a race for a
        // filename, and there is nowhere in this head to show a second
        // percentage anyway.
        if self.fetching.running.swap(true, Ordering::Relaxed) {
            return;
        }

        let link = link.trim().to_owned();
        let library = Arc::clone(&self.services.library);
        let state = Arc::clone(&self.fetching);

        state.percent.store(0, Ordering::Relaxed);
        state.item.store(0, Ordering::Relaxed);
        state.of.store(0, Ordering::Relaxed);
        state.stopping.store(false, Ordering::Relaxed);
        *state
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;

        if let Some(window) = self.window.upgrade() {
            window.global::<Fetch>().set_fetching(true);
            window.global::<Fetch>().set_percent(0);
            window.global::<Fetch>().set_needs_folder(false);
            // Said before anything has happened, because `yt-dlp` takes a
            // second or two to answer and a button that goes quiet reads as a
            // button that did not work.
            window
                .global::<Fetch>()
                .set_note(crate::text::tr("reaching for it…").into());
        }

        std::thread::spawn(move || {
            let watching = Arc::clone(&state);
            let ended = match library.fetch_from_link(
                &link,
                what,
                &|report| {
                    state.percent.store(report.percent, Ordering::Relaxed);
                    let (item, of) = report.item.unwrap_or_default();
                    state.item.store(item, Ordering::Relaxed);
                    state.of.store(of, Ordering::Relaxed);
                },
                &move || watching.stopping.load(Ordering::Relaxed),
            ) {
                Ok(Fetched::Landed(name)) => {
                    Ended::Landed(crate::text::tr1("{} - in your library", &name))
                }
                Ok(Fetched::LandedMany(count)) => Ended::Landed(crate::text::tr1(
                    "{} - in your library",
                    &library_vm::counted_tracks(count),
                )),
                // Two things arrive here and the difference is worth saying:
                // one is a listener who pressed stop, the other is a link
                // whose tracks are already here. Both leave the library as it
                // was, and neither is a failure.
                Ok(Fetched::NothingNew) => Ended::Landed(
                    crate::text::tr(
                        "nothing new - everything on that link is already in your library",
                    )
                    .to_owned(),
                ),
                Ok(Fetched::NeedsLocalFolder(path)) => Ended::NeedsFolder(crate::text::tr1(
                    "a track needs somewhere to land - Cadenza can make {}",
                    &path.display().to_string(),
                )),
                // Not a dead end any more. What is missing is named, and the
                // button beside it installs exactly that.
                Ok(Fetched::NeedsTools(missing)) => {
                    Ended::Offer(library_vm::tools_needed(&missing), Offer::Install)
                }
                Ok(Fetched::NeedsUpdate(said)) => Ended::Offer(
                    crate::text::tr1("{} - yt-dlp has probably fallen behind", &said),
                    Offer::Update,
                ),
                Err(err) => Ended::Failed(err.to_string()),
            };

            *state
                .finished
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(ended);
            // Last, so that whoever sees `running` false also sees the answer.
            state.running.store(false, Ordering::Relaxed);
        });
    }

    /// Reads how far a fetch has got, and what it came to in the end.
    ///
    /// On the tick, like everything else that happens off the event loop.
    pub fn poll_fetch(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let fetch_ui = window.global::<Fetch>();

        if self.fetching.running.load(Ordering::Relaxed) {
            fetch_ui.set_percent(i32::from(self.fetching.percent.load(Ordering::Relaxed)));

            // "3 of 40" while a playlist is coming, because a percentage that
            // goes back to nothing forty times answers no question anybody has.
            //
            // And "3" alone where that is all anybody knows: the matcher does
            // not say how many it is going to find, so what is counted is what
            // has arrived. A number that is true is better than a total that
            // was guessed.
            let of = self.fetching.of.load(Ordering::Relaxed);
            let item = self.fetching.item.load(Ordering::Relaxed);
            if !self.fetching.stopping.load(Ordering::Relaxed) {
                if of > 0 {
                    fetch_ui.set_note(
                        format!(
                            "{} {item} {} {of}",
                            crate::text::tr("track|of"),
                            crate::text::tr("of|found")
                        )
                        .into(),
                    );
                } else if item > 0 {
                    fetch_ui.set_note(
                        crate::text::tr1("{} so far", &library_vm::counted_tracks(item as usize))
                            .into(),
                    );
                }
            }
            return;
        }

        // Taken rather than read: this runs four times a second and the end of
        // a fetch is one event.
        let ended = self
            .fetching
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();

        let Some(ended) = ended else {
            return;
        };

        fetch_ui.set_fetching(false);
        fetch_ui.set_percent(0);

        match ended {
            Ended::Landed(said) => {
                fetch_ui.set_note(said.into());
                fetch_ui.set_needs_folder(false);
                // Emptied on the way in rather than left to be pressed again:
                // the link has been used, and a field still holding it is an
                // invitation to fetch the same track twice.
                fetch_ui.set_link(String::new().into());
                self.after_library_change();
            }
            Ended::NeedsFolder(said) => {
                fetch_ui.set_note(said.into());
                fetch_ui.set_needs_folder(true);
            }
            Ended::Failed(said) => {
                fetch_ui.set_note(said.into());
                fetch_ui.set_needs_folder(false);
                self.offer.set(None);
                fetch_ui.set_offer(String::new().into());
            }
            Ended::Offer(said, offer) => {
                fetch_ui.set_note(said.into());
                fetch_ui.set_needs_folder(false);
                self.offer.set(Some(offer));
                fetch_ui.set_offer(offer.label().into());
            }
            // Straight on to the thing that failed, rather than asking the
            // listener to press the button they already pressed: they said
            // what they wanted, this was the obstacle, and the obstacle is
            // gone.
            Ended::Fixed(said) => {
                fetch_ui.set_note(said.into());
                self.offer.set(None);
                fetch_ui.set_offer(String::new().into());

                let link = fetch_ui.get_link().to_string();
                if !link.trim().is_empty() {
                    self.fetch(&link, self.asked_for.get());
                }
            }
        }
    }

    /// Does the one thing that would make the last press work, and then makes
    /// that press again.
    ///
    /// Installing through the machine's own package manager, or updating
    /// through the downloader's own updater. Never without being asked: this
    /// runs from a button that says what it is about to do.
    pub fn fix_fetch(&self) {
        let Some(offer) = self.offer.get() else {
            return;
        };
        if self.fetching.running.swap(true, Ordering::Relaxed) {
            return;
        }

        let library = Arc::clone(&self.services.library);
        let state = Arc::clone(&self.fetching);
        // The link that failed, because what a link needs depends on the link:
        // a Spotify address wants the program that reads its names, and a
        // listener who only pastes YouTube links must never be told to install
        // that one.
        let link = self
            .window
            .upgrade()
            .map(|window| window.global::<Fetch>().get_link().to_string())
            .unwrap_or_default();

        state.percent.store(0, Ordering::Relaxed);
        state.item.store(0, Ordering::Relaxed);
        state.of.store(0, Ordering::Relaxed);
        *state
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;

        if let Some(window) = self.window.upgrade() {
            window.global::<Fetch>().set_fetching(true);
            window.global::<Fetch>().set_offer(String::new().into());
            window.global::<Fetch>().set_note(
                match offer {
                    Offer::Install => crate::text::tr("installing what this needs…"),
                    Offer::Update => crate::text::tr("updating yt-dlp…"),
                }
                .into(),
            );
        }

        std::thread::spawn(move || {
            let ended = match offer {
                Offer::Install => match library.install_tools(&link, &|_| {}) {
                    Ok(missing) if missing.is_empty() => {
                        Ended::Fixed(crate::text::tr("installed - trying again").to_owned())
                    }
                    Ok(missing) => Ended::Failed(library_vm::tools_needed(&missing)),
                    Err(err) => Ended::Failed(err.to_string()),
                },
                Offer::Update => match library.update_downloader(&|_| {}) {
                    Ok(spoke) => Ended::Fixed(crate::text::tr1("{} - trying again", &spoke)),
                    Err(err) => Ended::Failed(err.to_string()),
                },
            };
            *state
                .finished
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(ended);
            state.running.store(false, Ordering::Relaxed);
        });
    }
}
