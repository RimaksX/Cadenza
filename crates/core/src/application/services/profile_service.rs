//! Creating, switching and deleting listener profiles.

use std::sync::Arc;

use crate::application::context::{ACTIVE_PROFILE_KEY, AppContext};
use crate::domain::ids::ProfileId;
use crate::domain::ports::event_bus::DomainEvent;
use crate::domain::profile::{Profile, ProfileName};
use crate::domain::settings::SettingValue;
use crate::domain::value_objects::ThemeMode;
use crate::{CoreError, Result};

/// The profile use cases.
pub struct ProfileService {
    context: Arc<AppContext>,
}

impl ProfileService {
    /// Wraps the shared context.
    pub fn new(context: Arc<AppContext>) -> Self {
        Self { context }
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
    /// History stays off until the setup wizard asks (PROJECT_MASTER 2.6). The
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
    /// purging is the analytics service's job in M14, and doing it here would
    /// mean this service reaching into a repository it has no other reason to
    /// know about. Nothing further is written while the flag is off.
    pub fn set_history_enabled(&self, id: ProfileId, enabled: bool) -> Result<Profile> {
        let mut profile = self.get(id)?;
        profile.history_enabled = enabled;
        self.context.profiles.save(&profile)?;
        Ok(profile)
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
    /// PROJECT_MASTER 2.5 defines three steps: stop playback, save the outgoing
    /// profile's state, load the incoming one's. Only the third is possible
    /// today — the first two need the playback and queue ports, which arrive in
    /// M5 and M7. When they do, they hook in here, before the pointer moves.
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
