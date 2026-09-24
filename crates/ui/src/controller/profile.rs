//! Who is listening: switching, making and removing listeners, and their pictures.

use super::*;

impl Controller {
    /// Who is listening, and in which theme.
    pub(super) fn refresh_profile(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        // Which decides whether there is an application at all yet: with nobody
        // created, the shell is replaced by the one screen that asks.
        window.set_first_run(self.profile.borrow().is_none());

        match self.profile.borrow().as_ref() {
            Some(profile) => {
                window.set_profile_name(profile.name.as_str().into());
                window
                    .global::<Theme>()
                    .set_dark(profile.theme == ThemeMode::Dark);
            }
            None => {
                window.set_profile_name("nobody".into());
                window.global::<Theme>().set_dark(true);
            }
        }
    }

    /// A new name for whoever is listening: the one Cadenza greets.
    pub fn rename_profile(&self, name: &str) {
        let Some(profile_id) = self.profile.borrow().as_ref().map(|profile| profile.id) else {
            return;
        };
        self.run(|| {
            let profile = self.services.profiles.rename(profile_id, name)?;
            *self.profile.borrow_mut() = Some(profile);
            Ok(())
        });
        self.refresh_profile();
        self.refresh_settings();
    }

    /// Makes the listener just created the one everything belongs to.
    ///
    /// Three steps in order - playback stops, the listener becomes the active
    /// one, their queue, station and level are loaded - and the order is the
    /// whole of it. Only the first run comes here: there is one listener, and
    /// this is where they begin.
    fn begin_with(&self, profile_id: ProfileId) {
        self.run(|| {
            self.services.playback.stop()?;

            let profile = self.services.profiles.switch_to(profile_id)?;
            self.services.queue.reload();
            self.services.radio.stop();
            self.services.playback.restore_volume()?;

            *self.profile.borrow_mut() = Some(profile);
            Ok(())
        });

        self.refresh_all();
    }

    /// The first run, answered: a listener, and somewhere to put music.
    ///
    /// Both already exist as commands on the settings screen; this is the order
    /// they are needed in the first time, which is the only thing the welcome
    /// screen adds. The folder comes second because it belongs to a profile,
    /// and is made only if it was asked for — a player that writes to somebody's
    /// disk unbidden has to be forgiven for it later.
    pub fn start_here(&self, name: &str, folder: bool, find: bool) {
        let created = self.services.profiles.create(name);
        let Ok(profile) = created else {
            if let Err(err) = created {
                self.report(&err);
            }
            return;
        };

        // Beginning refreshes everything, so the shell is standing before the
        // folder is made — and a failure there is then reported into a window
        // that can show it.
        self.begin_with(profile.id);

        if folder {
            self.use_suggested_folder();
        }
        // Asked for on the welcome card, and done once there is a library to
        // put it in: the chooser, for the music this computer already holds.
        if find {
            self.add_folder();
        }
    }

    /// A picture from disk, decoded at most once.
    ///
    /// Emptied rather than invalidated whenever a picture is chosen or removed:
    /// the map holds a handful of entries, and working out which one changed
    /// costs more thought than dropping all of them costs time.
    pub(super) fn picture(&self, path: &Path) -> slint::Image {
        if let Some(known) = self.pictures.borrow().get(path).cloned() {
            return known;
        }

        let picture = slint::Image::load_from_path(path).unwrap_or_default();
        self.pictures
            .borrow_mut()
            .insert(path.to_path_buf(), picture.clone());
        picture
    }

    /// A track's cover at the size a row draws, from the cache the rows keep.
    pub(super) fn thumbnail(&self, id: MediaFileId) -> slint::Image {
        if let Some(known) = self.covers.borrow().get(&id).cloned() {
            return known;
        }
        let Some(image) = self
            .services
            .library
            .thumbnail_for(id)
            .ok()
            .flatten()
            .and_then(|path| slint::Image::load_from_path(&path).ok())
        else {
            return slint::Image::default();
        };
        self.covers.borrow_mut().insert(id, image.clone());
        image
    }

    /// Forgets every decoded picture, because one of them is no longer what it
    /// was.
    pub(super) fn forget_pictures(&self) {
        self.pictures.borrow_mut().clear();
        self.tints.borrow_mut().clear();
    }

    /// The colour of a picture from disk, worked out at most once.
    pub(super) fn tint_for(&self, path: &Path) -> Option<slint::Color> {
        if let Some(known) = self.tints.borrow().get(path) {
            return *known;
        }
        let tint = tint_of(&self.picture(path));
        self.tints.borrow_mut().insert(path.to_path_buf(), tint);
        tint
    }
}
