//! Shared dependencies and the active profile.

use std::sync::{Arc, RwLock};

use crate::domain::ids::ProfileId;
use crate::domain::ports::clock::ClockPort;
use crate::domain::ports::event_bus::{DomainEvent, EventBusPort};
use crate::domain::ports::repositories::{ProfileRepositoryPort, SettingsRepositoryPort};
use crate::domain::value_objects::Timestamp;
use crate::{CoreError, Result};

/// Key under which the active profile is stored in `app_settings`.
///
/// Global rather than per-profile, and one of the few pieces of state that is
/// not user data: it records which listener the application starts as
/// (PROJECT_MASTER 2.5).
pub const ACTIVE_PROFILE_KEY: &str = "active_profile_id";

/// What every application service is handed.
///
/// Holds only the ports that exist today. It grows one field per milestone as
/// infrastructure lands, rather than declaring twenty ports now and forcing M2
/// to stub every one of them just to construct this.
pub struct AppContext {
    /// The clock. Nothing in the application layer reads the system time
    /// directly.
    pub clock: Arc<dyn ClockPort>,
    /// Where change notifications go.
    pub events: Arc<dyn EventBusPort>,
    /// Profile storage.
    pub profiles: Arc<dyn ProfileRepositoryPort>,
    /// Application and profile settings.
    pub settings: Arc<dyn SettingsRepositoryPort>,
    /// Who is listening right now.
    ///
    /// Behind a lock because background workers read it while the UI thread may
    /// be switching it. Held only for the length of a read or a write, never
    /// across a repository call.
    active_profile: RwLock<Option<ProfileId>>,
}

impl AppContext {
    /// Assembles the context with no profile selected yet.
    pub fn new(
        clock: Arc<dyn ClockPort>,
        events: Arc<dyn EventBusPort>,
        profiles: Arc<dyn ProfileRepositoryPort>,
        settings: Arc<dyn SettingsRepositoryPort>,
    ) -> Self {
        Self {
            clock,
            events,
            profiles,
            settings,
            active_profile: RwLock::new(None),
        }
    }

    /// The current time, from the injected clock.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Who is listening, if anyone.
    pub fn active_profile(&self) -> Option<ProfileId> {
        self.read_active()
    }

    /// Who is listening, or an error.
    ///
    /// Anything touching user data goes through this rather than through
    /// [`Self::active_profile`], so that "no profile selected" surfaces as a
    /// clear failure instead of silently reading nothing.
    pub fn require_active_profile(&self) -> Result<ProfileId> {
        self.read_active().ok_or(CoreError::NoActiveProfile)
    }

    /// Switches profiles and announces it.
    ///
    /// Only the in-memory pointer and the notification. Stopping playback and
    /// saving the outgoing profile's state — steps 1 and 2 of the switching
    /// procedure in PROJECT_MASTER 2.5 — are the profile service's job in M3,
    /// because they need the playback and queue ports this context does not
    /// carry yet.
    pub fn set_active_profile(&self, profile_id: ProfileId) {
        if let Ok(mut active) = self.active_profile.write() {
            *active = Some(profile_id);
        }
        self.events
            .publish(DomainEvent::ProfileSwitched(profile_id));
    }

    /// Leaves no profile active.
    ///
    /// Used when the active profile is deleted. No event: nothing has been
    /// switched *to*, and a subscriber told "the profile changed" with no
    /// profile to load would have nothing useful to do.
    pub fn clear_active_profile(&self) {
        if let Ok(mut active) = self.active_profile.write() {
            *active = None;
        }
    }

    /// Reads the active profile, treating a poisoned lock as "nobody".
    ///
    /// A poisoned lock means another thread panicked mid-switch. Refusing to
    /// name a profile is the safe answer: the alternative is writing one
    /// listener's data under another's identity.
    fn read_active(&self) -> Option<ProfileId> {
        self.active_profile.read().ok().and_then(|active| *active)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::AppContext;
    use crate::CoreError;
    use crate::domain::ids::ProfileId;
    use crate::domain::ports::clock::ClockPort;
    use crate::domain::ports::event_bus::{DomainEvent, EventBusPort, EventHandler};
    use crate::domain::ports::repositories::{ProfileRepositoryPort, SettingsRepositoryPort};
    use crate::domain::profile::Profile;
    use crate::domain::settings::{ProfileFolder, SettingValue};
    use crate::domain::value_objects::Timestamp;

    struct FixedClock(Timestamp);
    impl ClockPort for FixedClock {
        fn now(&self) -> Timestamp {
            self.0
        }
    }

    #[derive(Default)]
    struct RecordingBus(Mutex<Vec<DomainEvent>>);
    impl EventBusPort for RecordingBus {
        fn publish(&self, event: DomainEvent) {
            self.0.lock().expect("not poisoned").push(event);
        }
        fn subscribe(&self, _handler: EventHandler) {}
    }

    struct NoProfiles;
    impl ProfileRepositoryPort for NoProfiles {
        fn list(&self) -> crate::Result<Vec<Profile>> {
            Ok(Vec::new())
        }
        fn get(&self, _id: ProfileId) -> crate::Result<Option<Profile>> {
            Ok(None)
        }
        fn save(&self, _profile: &Profile) -> crate::Result<()> {
            Ok(())
        }
        fn delete(&self, _id: ProfileId) -> crate::Result<()> {
            Ok(())
        }
    }

    struct NoSettings;
    impl SettingsRepositoryPort for NoSettings {
        fn app_get(&self, _key: &str) -> crate::Result<Option<SettingValue>> {
            Ok(None)
        }
        fn app_set(&self, _key: &str, _value: &SettingValue, _now: Timestamp) -> crate::Result<()> {
            Ok(())
        }
        fn app_remove(&self, _key: &str) -> crate::Result<()> {
            Ok(())
        }
        fn profile_get(&self, _p: ProfileId, _key: &str) -> crate::Result<Option<SettingValue>> {
            Ok(None)
        }
        fn profile_set(
            &self,
            _p: ProfileId,
            _key: &str,
            _value: &SettingValue,
            _now: Timestamp,
        ) -> crate::Result<()> {
            Ok(())
        }
        fn profile_remove(&self, _p: ProfileId, _key: &str) -> crate::Result<()> {
            Ok(())
        }
        fn list_folders(&self, _p: ProfileId) -> crate::Result<Vec<ProfileFolder>> {
            Ok(Vec::new())
        }
        fn save_folder(&self, _folder: &ProfileFolder) -> crate::Result<()> {
            Ok(())
        }
        fn delete_folder(&self, _folder: &ProfileFolder) -> crate::Result<()> {
            Ok(())
        }
    }

    fn context(bus: Arc<RecordingBus>) -> AppContext {
        AppContext::new(
            Arc::new(FixedClock(Timestamp::from_millis(1_754_611_200_000))),
            bus,
            Arc::new(NoProfiles),
            Arc::new(NoSettings),
        )
    }

    #[test]
    fn time_comes_from_the_injected_clock() {
        let context = context(Arc::new(RecordingBus::default()));
        assert_eq!(context.now(), Timestamp::from_millis(1_754_611_200_000));
    }

    #[test]
    fn user_data_access_fails_loudly_with_no_profile_selected() {
        let context = context(Arc::new(RecordingBus::default()));
        assert_eq!(context.active_profile(), None);
        assert!(matches!(
            context.require_active_profile(),
            Err(CoreError::NoActiveProfile)
        ));
    }

    #[test]
    fn switching_profiles_announces_the_change() {
        let bus = Arc::new(RecordingBus::default());
        let context = context(Arc::clone(&bus));
        let profile = ProfileId::new();

        context.set_active_profile(profile);

        assert_eq!(context.require_active_profile().expect("selected"), profile);
        assert_eq!(
            *bus.0.lock().expect("not poisoned"),
            vec![DomainEvent::ProfileSwitched(profile)]
        );
    }
}
