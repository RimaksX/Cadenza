//! Creating, switching and deleting listener profiles.

use std::sync::Arc;

use super::cover;
use crate::application::context::{ACTIVE_PROFILE_KEY, AppContext};
use crate::domain::ids::ProfileId;
use crate::domain::ports::artwork_cache::CoverOf;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::profile::{Profile, ProfileName};
use crate::domain::settings::{InterfaceScale, SettingValue, UI_SCALE_KEY};
use crate::domain::value_objects::ThemeMode;
use crate::{CoreError, Result};

/// The profile use cases.
pub struct ProfileService {
    context: Arc<AppContext>,
    /// What a picture needs to be chosen and kept.
    ///
    /// `None` where the caller has no interface to choose one with — every test
    /// in the suite, and anything that only reads profiles. Asking for an
    /// avatar without them is a programming mistake rather than a listener's,
    /// and it says so.
    covers: Option<cover::CoverPorts>,
}

impl ProfileService {
    /// A service that can put a face on a profile as well as read one.
    pub fn with_covers(context: Arc<AppContext>, covers: cover::CoverPorts) -> Self {
        Self {
            context,
            covers: Some(covers),
        }
    }

    /// The same, for a caller with no way to choose a picture: every test in
    /// the suite, and anything that only reads profiles.
    pub fn new(context: Arc<AppContext>) -> Self {
        Self {
            context,
            covers: None,
        }
    }

    /// Every profile, ordered by name.
    pub fn list(&self) -> Result<Vec<Profile>> {
        self.context.profiles.list()
    }

    /// One profile, or an error naming what was not found.
    pub fn get(&self, id: ProfileId) -> Result<Profile> {
        self.context
            .profiles
            .get(id)?
            .ok_or_else(|| CoreError::not_found("profile", id))
    }

    /// Creates a profile with history disabled.
    ///
    /// History stays off until the setup wizard asks. The
    /// safe default is the one that records nothing: a listener who never
    /// answers the question ends up with no history rather than with history
    /// they did not agree to.
    ///
    /// The first profile created becomes the active one, so a fresh install is
    /// usable without a separate "now choose a profile" step.
    ///
    /// "First" is judged from the stored pointer rather than from the in-memory
    /// one. Otherwise a caller that skipped [`Self::restore_active`] would see an
    /// empty context, decide this profile is the first, and quietly reassign the
    /// active slot away from whoever was using it.
    pub fn create(&self, name: &str) -> Result<Profile> {
        let name = ProfileName::new(name)?;
        let profile = Profile::new(name, self.context.now());

        self.context.profiles.save(&profile)?;

        if self.context.settings.app_get(ACTIVE_PROFILE_KEY)?.is_none() {
            self.activate(profile.id)?;
        }

        Ok(profile)
    }

    /// Renames a profile.
    pub fn rename(&self, id: ProfileId, name: &str) -> Result<Profile> {
        let mut profile = self.get(id)?;
        profile.name = ProfileName::new(name)?;
        self.context.profiles.save(&profile)?;
        Ok(profile)
    }

    /// Turns listening history on or off.
    ///
    /// Turning it off does not by itself erase what was already recorded —
    /// purging is the analytics service's job, and doing it here would
    /// mean this service reaching into a repository it has no other reason to
    /// know about. Nothing further is written while the flag is off.
    pub fn set_history_enabled(&self, id: ProfileId, enabled: bool) -> Result<Profile> {
        let mut profile = self.get(id)?;
        profile.history_enabled = enabled;
        self.context.profiles.save(&profile)?;
        Ok(profile)
    }

    /// How large this profile draws the interface.
    ///
    /// A stored value that is not one of the steps on offer is treated as
    /// never having chosen: the sizes are a closed set, and a row edited by
    /// hand is not a reason to draw the window at 400 per cent.
    pub fn interface_scale(&self, id: ProfileId) -> Result<InterfaceScale> {
        let stored = self.context.settings.profile_get(id, UI_SCALE_KEY)?;
        Ok(stored
            .and_then(|value| value.as_integer().ok())
            .and_then(|percent| u16::try_from(percent).ok())
            .and_then(|percent| InterfaceScale::new(percent).ok())
            .unwrap_or_default())
    }

    /// Chooses how large this profile draws the interface.
    pub fn set_interface_scale(&self, id: ProfileId, scale: InterfaceScale) -> Result<()> {
        self.context.settings.profile_set(
            id,
            UI_SCALE_KEY,
            &SettingValue::Integer(i64::from(scale.percent())),
            self.context.now(),
        )
    }

    /// Changes the appearance a profile uses.
    pub fn set_theme(&self, id: ProfileId, theme: ThemeMode) -> Result<Profile> {
        let mut profile = self.get(id)?;
        profile.theme = theme;
        self.context.profiles.save(&profile)?;
        Ok(profile)
    }

    /// Switches the active profile.
    ///
    /// Switching profiles is three steps: stop playback, save the outgoing
    /// profile's state, load the incoming one's. Only the third is possible
    /// today — the first two need the playback and queue ports. When those are
    /// wired up, they hook in here, before the pointer moves.
    ///
    /// The profile must exist. Pointing the application at a deleted profile
    /// would leave every subsequent query returning nothing with no explanation.
    pub fn switch_to(&self, id: ProfileId) -> Result<Profile> {
        let profile = self.get(id)?;
        self.activate(id)?;
        Ok(profile)
    }

    /// Restores the profile that was active when the application last closed.
    ///
    /// Returns `None` when there is nothing to restore: a fresh install, or a
    /// pointer left behind by a profile that has since been deleted. A stale
    /// pointer is cleaned up rather than reported, because there is nothing the
    /// listener could do about it.
    pub fn restore_active(&self) -> Result<Option<Profile>> {
        let Some(stored) = self.context.settings.app_get(ACTIVE_PROFILE_KEY)? else {
            return Ok(None);
        };

        let id = ProfileId::parse(stored.as_text()?)?;

        let Some(profile) = self.context.profiles.get(id)? else {
            self.context.settings.app_remove(ACTIVE_PROFILE_KEY)?;
            return Ok(None);
        };

        self.context.set_active_profile(profile.id);
        Ok(Some(profile))
    }

    /// Puts a picture beside a listener's name.
    ///
    /// `Ok(false)` means the chooser was closed, which is an answer.
    ///
    /// Nothing is published. Renaming a profile does not publish either: the
    /// events here are coarse and name an *area*, and there is no area whose
    /// subscribers would re-read a portrait. The screen that asked refreshes
    /// itself, which is what it does after a rename.
    pub fn choose_avatar(&self, id: ProfileId) -> Result<bool> {
        let ports = self.covers()?;
        // Read first, so a picture cannot be hung on a profile that is no
        // longer there.
        self.get(id)?;
        cover::choose(ports, CoverOf::Profile(id), "Choose a picture")
    }

    /// Takes the picture off again, leaving the initial that stood there first.
    pub fn clear_avatar(&self, id: ProfileId) -> Result<()> {
        let ports = self.covers()?;
        self.get(id)?;
        cover::clear(ports, CoverOf::Profile(id))
    }

    /// Where a listener's picture is, if they chose one.
    pub fn avatar(&self, id: ProfileId) -> Option<std::path::PathBuf> {
        self.covers
            .as_ref()
            .and_then(|ports| ports.artwork.path_for(CoverOf::Profile(id)))
    }

    fn covers(&self) -> Result<&cover::CoverPorts> {
        self.covers
            .as_ref()
            .ok_or_else(|| CoreError::invalid("profile picture", "this build cannot choose one"))
    }

    /// Deletes a profile and everything scoped to it.
    ///
    /// The database cascades the profile's library membership, playlists,
    /// history, presets and settings. Files on disk and the shared catalogue are
    /// untouched — other profiles keep their copies.
    ///
    /// Deleting the active profile clears the pointer rather than refusing.
    /// Refusing would make the last profile undeletable, and silently switching
    /// to another one would be a decision this layer has no business making.
    pub fn delete(&self, id: ProfileId) -> Result<()> {
        // Before the row, because after it there is nothing left to name the
        // file by. A failure here does not stop the deletion: the picture is a
        // file in a cache, and one stale file is a smaller problem than a
        // listener who asked to be forgotten and was not.
        if let Some(ports) = self.covers.as_ref()
            && let Err(err) = cover::clear(ports, CoverOf::Profile(id))
        {
            self.context
                .warn(&format!("a profile's picture was left behind: {err}"));
        }

        self.context.profiles.delete(id)?;

        if self.context.active_profile() == Some(id) {
            self.context.settings.app_remove(ACTIVE_PROFILE_KEY)?;
            self.context.clear_active_profile();
        }

        self.context.events.publish(DomainEvent::LibraryChanged);
        Ok(())
    }

    /// Persists the pointer and tells the context, in that order.
    ///
    /// Persisting first means a crash between the two leaves the stored pointer
    /// correct; the reverse order would leave the running application believing
    /// in a switch that the next start would not honour.
    fn activate(&self, id: ProfileId) -> Result<()> {
        self.context.settings.app_set(
            ACTIVE_PROFILE_KEY,
            &SettingValue::Text(id.to_string()),
            self.context.now(),
        )?;
        self.context.set_active_profile(id);
        Ok(())
    }
}
