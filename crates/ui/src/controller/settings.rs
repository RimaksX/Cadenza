//! The settings page: preferences and the folders the library is kept in.

use super::*;

impl Controller {
    /// Everything the settings screen draws.
    pub fn refresh_settings(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let settings_ui = window.global::<Settings>();

        if let Some(profile) = self.profile.borrow().as_ref() {
            settings_ui.set_dark(profile.theme == ThemeMode::Dark);
            settings_ui.set_history_on(profile.history_enabled);
        }

        if let Ok(settings) = self.services.playback.settings() {
            settings_ui.set_normalise_on(settings.normalise);
            settings_ui.set_crossfade_on(settings.crossfade_enabled);
            settings_ui.set_crossfade_seconds(
                (settings.crossfade.as_duration().as_millis() / 1_000) as i32,
            );
        }

        settings_ui.set_suggested_folder(
            self.services
                .library
                .suggested_folder()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
                .into(),
        );

        let Ok(folders) = self.services.library.folders() else {
            return;
        };
        let rows: Vec<FolderRowData> = folders
            .iter()
            .map(|folder| FolderRowData {
                id: folder.id.to_string().into(),
                path: folder.path.display().to_string().into(),
                note: if folder.include_subfolders {
                    crate::text::tr("WITH SUBFOLDERS").into()
                } else {
                    crate::text::tr("THIS FOLDER ONLY").into()
                },
            })
            .collect();
        settings_ui.set_folders(ModelRc::new(VecModel::from(rows)));

        // What this listener took out, and how many tracks no folder looks
        // after. Both are read here rather than on a screen of their own: they
        // are facts about the library, and the library lives on this page.
        let taken_out: Vec<TakenOutRowData> = self
            .services
            .library
            .taken_out()
            .unwrap_or_default()
            .iter()
            .map(|summary| TakenOutRowData {
                id: summary.media_file_id.to_string().into(),
                title: summary.title.as_str().into(),
                subtitle: summary
                    .artist
                    .as_deref()
                    .unwrap_or(crate::text::tr("Unknown artist"))
                    .into(),
                gone: !self.services.library.is_on_disk(summary.media_file_id),
            })
            .collect();
        settings_ui.set_taken_out(ModelRc::new(VecModel::from(taken_out)));
        settings_ui.set_gone_for_good(
            i32::try_from(self.services.library.gone_for_good().unwrap_or(0)).unwrap_or(0),
        );
        settings_ui.set_outside_folders(
            i32::try_from(self.services.library.outside_folders().unwrap_or(0)).unwrap_or(0),
        );

        // A greeting rather than a count. How many folders and tracks there
        // are is a fact about the library, and the library has a page that
        // says it; here it answered a question nobody had come to ask. What
        // this page does need to say at the top is *whose* settings these are,
        // because every one of them is kept per profile — a size, a theme, a
        // crossfade and a history belong to the listener, not to the machine.
        settings_ui.set_summary(
            self.profile
                .borrow()
                .as_ref()
                .map(|profile| crate::text::tr1("Welcome, {}", profile.name.as_str()))
                .unwrap_or_default()
                .into(),
        );
    }

    /// Which way round the ink and the paper go.
    pub fn set_theme(&self, dark: bool) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        let mode = if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };

        self.run(|| {
            let updated = self.services.profiles.set_theme(profile.id, mode)?;
            *self.profile.borrow_mut() = Some(updated);
            Ok(())
        });

        if let Some(window) = self.window.upgrade() {
            window.global::<Theme>().set_dark(dark);
        }
        self.refresh_settings();
    }

    /// Whether loud tracks are turned down to sit with the rest.
    pub fn set_normalise(&self, enabled: bool) {
        self.run(|| self.services.playback.set_normalise(enabled));
        self.refresh_settings();
    }

    /// Plays at another speed, chosen on the line as a percentage.
    pub fn set_speed(&self, percent: i32) {
        let Ok(percent) = u16::try_from(percent) else {
            return;
        };
        self.run(|| {
            let speed = PlaybackSpeed::new(percent)?;
            self.services.playback.set_speed(speed)
        });
    }

    /// Whether ordinary tracks fade into each other, and over how long.
    pub fn set_crossfade(&self, enabled: bool, seconds: i32) {
        self.run(|| {
            let duration = CrossfadeDuration::new(DurationMs::from_secs(seconds.max(0) as u64))?;
            self.services.playback.set_crossfade(enabled, duration)
        });
        self.refresh_settings();
    }

    /// Whether what was played is written down.
    pub fn set_history(&self, keep: bool) {
        let Some(profile) = self.profile.borrow().clone() else {
            return;
        };
        self.run(|| {
            let updated = self
                .services
                .profiles
                .set_history_enabled(profile.id, keep)?;

            // Off means there is nothing written, not "stop writing from now
            // on". A month of listening left sitting behind a switch that says
            // off is exactly what the retention rule refuses, and the service
            // that owns retention is the one that erases it.
            if !keep {
                self.services.stats.forget(profile.id)?;
            }

            *self.profile.borrow_mut() = Some(updated);
            Ok(())
        });
        self.refresh_settings();
    }

    /// Asks for a folder and takes in what is in it.
    pub fn add_folder(&self) {
        match self.services.library.choose_folder() {
            Ok(Some(report)) => self.say(&library_vm::taken_in(&report)),
            Ok(None) => {}
            Err(err) => self.report(&err),
        }
        self.after_library_change();
    }

    /// Makes the folder this machine suggests and watches it.
    pub fn use_suggested_folder(&self) {
        self.run(|| self.services.library.use_suggested_folder().map(|_| ()));
        self.after_library_change();
    }

    /// Stops watching one. What was imported from it stays.
    pub fn remove_folder(&self, id: &str) {
        self.run(|| {
            let folders = self.services.library.folders()?;
            let folder = folders
                .iter()
                .find(|folder| folder.id.to_string() == id)
                .ok_or_else(|| CoreError::not_found("library folder", id))?;
            self.services.library.remove_folder(folder)
        });
        self.after_library_change();
    }

    /// Makes one folder and the library agree, and says what changed.
    pub fn synchronise_folder(&self, id: &str) {
        let said = match self.folder_by_id(id).and_then(|folder| {
            self.services
                .library
                .synchronise(&folder)
                .map(|report| library_vm::taken_in(&report))
        }) {
            Ok(said) => said,
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        self.say(&said);
        self.after_library_change();
    }

    /// Puts one track back into the library.
    pub fn restore_track(&self, id: &str) {
        let Ok(media_file_id) = MediaFileId::parse(id) else {
            return;
        };
        self.run(|| self.services.library.restore_track(media_file_id));
        self.after_library_change();
    }

    /// Takes every removal with no file behind it off the list for good.
    pub fn forget_gone(&self) {
        let said = match self.services.library.forget_gone() {
            Ok(count) => library_vm::forgotten(count),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        self.say(&said);
        self.after_library_change();
    }

    /// The folder one of the settings rows stands for.
    pub(super) fn folder_by_id(&self, id: &str) -> Result<ProfileFolder> {
        self.services
            .library
            .folders()?
            .into_iter()
            .find(|folder| folder.id.to_string() == id)
            .ok_or_else(|| CoreError::not_found("library folder", id))
    }

    /// Looks again at every folder, and says what it found.
    ///
    /// A scan that reports nothing looks the same whether it found nothing or
    /// was never run, which is exactly the doubt somebody presses it to settle.
    pub fn scan_now(&self) {
        let said = match self.services.library.scan_all() {
            Ok(report) => library_vm::taken_in(&report),
            Err(err) => {
                self.report(&err);
                return;
            }
        };

        self.say(&said);
        self.after_library_change();
    }

    /// Makes the folder Cadenza suggests, and carries on with what was asked.
    ///
    /// The listener said no to this folder once, which was a fair answer to a
    /// question about their disk asked for no particular reason. Now there is a
    /// reason, they have agreed, and making them press GET a second time would
    /// be asking them to confirm the thing they just confirmed.
    pub fn make_local_folder(&self) {
        if self.services.library.use_suggested_folder().is_err() {
            if let Some(window) = self.window.upgrade() {
                window
                    .global::<Fetch>()
                    .set_note(crate::text::tr("that folder could not be made").into());
            }
            return;
        }

        let link = self.window.upgrade().map(|window| {
            window.global::<Fetch>().set_needs_folder(false);
            window.global::<Fetch>().get_link().to_string()
        });

        self.after_library_change();

        if let Some(link) = link.filter(|link| !link.trim().is_empty()) {
            self.fetch_from_link(&link);
        }
    }

    /// Saves a copy of everything to a file the listener names.
    pub fn save_backup(&self) {
        match self.services.backup.save_copy() {
            Ok(Some(path)) => self.say(&crate::text::tr1(
                "A copy is saved as {}",
                &path
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            )),
            Ok(None) => {}
            Err(err) => self.report(&err),
        }
    }

    /// Sets a copy aside to replace everything, and closes the window so that
    /// Cadenza can open again on it. The place of the window is kept first, as
    /// any close keeps it.
    pub fn restore_backup(&self) {
        match self.services.backup.restore() {
            Ok(true) => {
                self.keep_placement();
                if let Some(window) = self.window.upgrade() {
                    let _ = window.hide();
                }
            }
            Ok(false) => {}
            Err(err) => self.report(&err),
        }
    }
}
