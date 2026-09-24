//! The window itself: its size, language and scale, its side columns, and which page is shown.

use super::*;

impl Controller {
    /// Draws the interface at the size this listener chose.
    ///
    /// The scale goes into the *lengths*, through `Theme.scale`, rather than
    /// into the renderer. A renderer told to draw at 1.25 puts every hairline
    /// on a pixel and a quarter and every stem between two columns, which is
    /// what a magnifying glass looks like; the lengths are rounded back onto
    /// whole pixels before anything is drawn.
    ///
    /// Which also makes it live. The window is not remade, it is re-measured.
    pub fn refresh_interface_scale(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let scale = self.chosen_scale();
        window
            .global::<Settings>()
            .set_ui_scale(i32::from(scale.percent()));
        window.global::<Theme>().set_scale(scale.factor());

        // And the frame grows with what is inside it. Lengths asking for a
        // tenth more room do not by themselves give it any: without this the
        // layout wants more than the window has, and the far side and the
        // bottom of every page are cut off — which is what a listener whose
        // size was not 100 per cent saw on every run after the one where they
        // chose it.
        let ratio = scale.factor() / self.sized_for.get();
        if (ratio - 1.0).abs() < f32::EPSILON {
            return;
        }
        self.sized_for.set(scale.factor());

        let size = window.window().size();
        window.window().set_size(slint::PhysicalSize::new(
            (size.width as f32 * ratio).round() as u32,
            (size.height as f32 * ratio).round() as u32,
        ));
    }

    /// How large this listener has asked for the interface to be drawn.
    pub(super) fn chosen_scale(&self) -> InterfaceScale {
        self.profile
            .borrow()
            .as_ref()
            .map(|profile| profile.id)
            .and_then(|id| self.services.profiles.interface_scale(id).ok())
            .unwrap_or_default()
    }

    /// Reads the chosen language and puts the interface into it.
    ///
    /// **Selecting a bundled translation needs a window already built**, which
    /// is why this is a refresh and not something `main` does before the event
    /// loop starts: before the first component exists there is no context for a
    /// bundle to be selected in, and the call fails.
    ///
    /// A tag the bundle does not carry leaves the interface in the language it
    /// was written in, which is what English is: there is no English catalogue,
    /// only the original strings.
    pub fn refresh_language(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let language = self.chosen_language();
        // An error here is the ordinary case for English, which has no
        // catalogue because it is what the markup already says. For anything
        // else it means the bundle was built without that language, and the
        // answer is the same either way: leave the original words up.
        let _ = slint::select_bundled_translation(language.tag());
        // The markup's half and the view models' half of the same catalogue.
        crate::text::select(language.tag());
        window
            .global::<Settings>()
            .set_language(language.tag().into());
    }

    /// What language this listener has asked for.
    pub(super) fn chosen_language(&self) -> Language {
        self.profile
            .borrow()
            .as_ref()
            .map(|profile| profile.id)
            .and_then(|id| self.services.profiles.language(id).ok())
            .unwrap_or_default()
    }

    /// Chooses the language, and redraws everything in it.
    ///
    /// Every string the window holds was formatted in the old language, so the
    /// whole window is filled again rather than only the markup's own words.
    pub fn set_language(&self, tag: &str) {
        let Some(profile) = self.profile.borrow().as_ref().map(|profile| profile.id) else {
            return;
        };
        let language = Language::from_tag(tag);
        self.run(|| self.services.profiles.set_language(profile, language));
        self.refresh_all();
    }

    /// Puts the window back where and how large it was left.
    ///
    /// Before the window is shown, and after the scale has sized it: the
    /// size kept is the one it had on screen, already at that scale.
    pub fn restore_placement(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Ok(Some(placement)) = self.services.profiles.window_placement() else {
            return;
        };
        window
            .window()
            .set_position(slint::PhysicalPosition::new(placement.x, placement.y));
        window
            .window()
            .set_size(slint::PhysicalSize::new(placement.width, placement.height));
        if placement.maximized {
            window.window().set_maximized(true);
            window.set_window_maximized(true);
        }
    }

    /// Keeps where the window is and how large, for the next start.
    ///
    /// A maximised window keeps the place it had before, so it comes back
    /// maximised and un-maximises to where it was. A minimised one is
    /// somewhere off every screen, and keeps nothing.
    pub fn keep_placement(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let frame = window.window();
        if frame.is_minimized() {
            return;
        }
        let maximized = frame.is_maximized();
        let placement = match self.services.profiles.window_placement() {
            Ok(Some(before)) if maximized => WindowPlacement {
                maximized,
                ..before
            },
            _ => {
                let position = frame.position();
                let size = frame.size();
                WindowPlacement {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                    maximized,
                }
            }
        };
        self.run(|| self.services.profiles.set_window_placement(placement));
    }

    /// Folds the side columns the way this listener left them.
    pub fn refresh_folds(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let profile = self.profile.borrow().as_ref().map(|profile| profile.id);
        let folded = |column| {
            profile
                .and_then(|id| self.services.profiles.folded(id, column).ok())
                .unwrap_or(false)
        };
        window.set_sidebar_folded(folded(SideColumn::Navigation));
        window.set_panel_folded(folded(SideColumn::NowPlaying));
        let rail = profile
            .and_then(|id| self.services.profiles.keeps_rail(id).ok())
            .unwrap_or(true);
        window.global::<Settings>().set_panel_rail(rail);
    }

    /// Folds the now-playing panel to its strip from now on, or away.
    pub fn set_panel_rail(&self, keep: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.global::<Settings>().set_panel_rail(keep);
        if let Some(profile) = self.profile.borrow().as_ref().map(|profile| profile.id) {
            self.run(|| self.services.profiles.set_keeps_rail(profile, keep));
        }
    }

    /// Folds a side column that is open, or opens one that is folded.
    pub fn toggle_fold(&self, column: SideColumn) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let folded = match column {
            SideColumn::Navigation => !window.get_sidebar_folded(),
            SideColumn::NowPlaying => !window.get_panel_folded(),
        };
        match column {
            SideColumn::Navigation => window.set_sidebar_folded(folded),
            SideColumn::NowPlaying => window.set_panel_folded(folded),
        }
        if let Some(profile) = self.profile.borrow().as_ref().map(|profile| profile.id) {
            self.run(|| self.services.profiles.set_folded(profile, column, folded));
        }
    }

    /// Chooses how large the interface is drawn, and draws it that way now.
    ///
    /// The frame grows with the lengths inside it, and that happens in
    /// [`Self::refresh_interface_scale`] rather than here — the window has to
    /// be given room whenever the scale is applied, and it is applied at every
    /// start as well as at every press.
    pub fn set_interface_scale(&self, percent: i32) {
        let Some(profile) = self.profile.borrow().as_ref().map(|profile| profile.id) else {
            return;
        };

        self.run(|| {
            let scale = InterfaceScale::new(u16::try_from(percent).unwrap_or_default())?;
            self.services.profiles.set_interface_scale(profile, scale)
        });
        self.refresh_interface_scale();
    }

    /// Re-reads what a change nobody in this window made could have altered.
    ///
    /// Three readings rather than all nine: a file appearing or vanishing moves
    /// the library, can take a track out of the queue, and can put a duplicate
    /// in front of the listener. It cannot change the equaliser, the theme or a
    /// month of listening, and re-reading those four times a second through a
    /// long import would be paid for in frames.
    pub fn refresh_after_change(&self) {
        self.refresh_library();
        self.refresh_queue();
        self.refresh_reviews();
    }

    /// Brings the page about to be shown up to date: a page built from a
    /// service reads it when asked, not when looked at.
    pub fn showing(&self, section: Section) {
        self.shown.set(section);
        match section {
            Section::Listening => self.refresh_listening(),
            Section::Reviews => self.refresh_reviews(),
            Section::Radio => self.refresh_radio(),
            Section::Settings => self.refresh_settings(),
            Section::Home => {
                self.refresh_home();
                self.refresh_queue();
            }
            Section::Playlists => self.refresh_playlists(),
            Section::Equaliser => self.refresh_eq(),
            Section::Library => self.refresh_library(),
            Section::Artists => self.refresh_artists(),
            // Filled by whatever opened it.
            Section::Artist => {}
            // Filled by whatever opened it.
            Section::Playlist => {}
        }
    }
}
