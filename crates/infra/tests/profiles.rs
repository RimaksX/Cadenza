//! Integration tests for profiles and settings: the M3 definition of done.
//!
//! These drive the real service over the real adapters against a real database.
//! The point is the seam between them — a service test with fake repositories
//! would prove the logic and nothing about whether a profile actually survives a
//! restart, which is the requirement.

use std::sync::{Arc, Mutex};

use cadenza_core::CoreError;
use cadenza_core::application::services::cover::CoverPorts;
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{ProfileFolderId, ProfileId};
use cadenza_core::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};
use cadenza_core::domain::ports::clock::ClockPort;
use cadenza_core::domain::ports::event_bus::{DomainEvent, EventBusPort};
use cadenza_core::domain::ports::folder_picker::FolderPickerPort;
use cadenza_core::domain::ports::repositories::SettingsRepositoryPort;
use cadenza_core::domain::settings::{ProfileFolder, SettingValue};
use cadenza_core::domain::value_objects::ThemeMode;
use cadenza_infra::db::repositories::{SqliteProfileRepository, SqliteSettingsRepository};
use cadenza_infra::events::InProcessEventBus;
use cadenza_infra::library::LocalFileSystem;
use cadenza_infra::metadata::FileArtworkCache;
use cadenza_testkit::{TempDb, TestClock};

/// Everything a test needs, wired the way the application wires it.
struct Harness {
    context: Arc<AppContext>,
    service: ProfileService,
    events: Arc<InProcessEventBus>,
    settings: Arc<SqliteSettingsRepository>,
    clock: Arc<TestClock>,
}

/// Builds a context over an existing database.
///
/// Separate from the fixture so that a test can build a *second* one over the
/// same file and observe what a restart would see.
fn attach(db: &TempDb) -> Harness {
    let clock = Arc::new(TestClock::default());
    let events = Arc::new(InProcessEventBus::new());
    let profiles = Arc::new(SqliteProfileRepository::new(db.pool().clone()));
    let settings = Arc::new(SqliteSettingsRepository::new(db.pool().clone()));

    let context = Arc::new(AppContext::new(
        Arc::clone(&clock) as _,
        Arc::clone(&events) as _,
        profiles,
        Arc::clone(&settings) as _,
    ));

    Harness {
        service: ProfileService::new(Arc::clone(&context)),
        context,
        events,
        settings,
        clock,
    }
}

impl Harness {
    /// Records every event published from now on.
    fn record_events(&self) -> Arc<Mutex<Vec<DomainEvent>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        self.events.subscribe(Box::new(move |event| {
            sink.lock().expect("not poisoned").push(event.clone());
        }));
        log
    }
}

fn fixture() -> (TempDb, Harness) {
    let db = TempDb::new();
    let harness = attach(&db);
    (db, harness)
}

#[test]
fn a_created_profile_is_stored_and_becomes_active() {
    let (_db, harness) = fixture();

    let profile = harness.service.create("Sasha").expect("a new profile");

    assert_eq!(profile.name.as_str(), "Sasha");
    assert!(
        !profile.history_enabled,
        "history stays off until the wizard asks"
    );
    assert_eq!(
        harness.context.active_profile(),
        Some(profile.id),
        "the first profile is the one to start as"
    );

    let stored = harness.service.get(profile.id).expect("it round-trips");
    assert_eq!(stored, profile);
}

#[test]
fn the_second_profile_does_not_steal_the_active_slot() {
    let (_db, harness) = fixture();

    let first = harness.service.create("Sasha").expect("first");
    harness.service.create("Kim").expect("second");

    assert_eq!(harness.context.active_profile(), Some(first.id));
}

#[test]
fn two_profiles_cannot_share_a_name() {
    let (_db, harness) = fixture();

    harness.service.create("Sasha").expect("first");
    let err = harness
        .service
        .create("Sasha")
        .expect_err("the name is taken");

    assert!(
        matches!(err, CoreError::Conflict(_)),
        "a taken name is a conflict the user can act on, got {err:?}"
    );
}

#[test]
fn an_invalid_name_never_reaches_the_database() {
    let (_db, harness) = fixture();

    assert!(harness.service.create("   ").is_err());
    assert!(
        harness.service.list().expect("listing").is_empty(),
        "a rejected name must not leave a row behind"
    );
}

#[test]
fn switching_persists_the_choice_and_announces_it() {
    let (_db, harness) = fixture();

    let first = harness.service.create("Sasha").expect("first");
    let second = harness.service.create("Kim").expect("second");
    let events = harness.record_events();

    harness.service.switch_to(second.id).expect("switching");

    assert_eq!(harness.context.active_profile(), Some(second.id));
    assert_ne!(second.id, first.id);
    assert_eq!(
        *events.lock().expect("not poisoned"),
        vec![DomainEvent::ProfileSwitched(second.id)]
    );
}

#[test]
fn switching_to_a_profile_that_does_not_exist_changes_nothing() {
    let (_db, harness) = fixture();

    let existing = harness.service.create("Sasha").expect("first");
    let err = harness
        .service
        .switch_to(ProfileId::new())
        .expect_err("there is no such profile");

    assert!(matches!(err, CoreError::NotFound { .. }), "got {err:?}");
    assert_eq!(
        harness.context.active_profile(),
        Some(existing.id),
        "a failed switch must leave the previous profile in place"
    );
}

#[test]
fn a_restart_comes_back_to_the_same_profile() {
    let db = TempDb::new();

    let created = {
        let first_run = attach(&db);
        first_run.service.create("Sasha").expect("first");
        let kim = first_run.service.create("Kim").expect("second");
        first_run.service.switch_to(kim.id).expect("switching");
        kim
    };

    // A fresh context over the same file: what the next start of the
    // application sees.
    let second_run = attach(&db);
    assert_eq!(
        second_run.context.active_profile(),
        None,
        "nothing is active until the pointer is read back"
    );

    let restored = second_run
        .service
        .restore_active()
        .expect("restoring")
        .expect("a profile was active");

    assert_eq!(restored.id, created.id);
    assert_eq!(restored.name.as_str(), "Kim");
    assert_eq!(second_run.context.active_profile(), Some(created.id));
}

#[test]
fn a_fresh_install_has_nothing_to_restore() {
    let (_db, harness) = fixture();
    assert!(
        harness
            .service
            .restore_active()
            .expect("restoring")
            .is_none()
    );
}

#[test]
fn a_pointer_left_by_a_deleted_profile_is_cleaned_up() {
    let db = TempDb::new();
    let first_run = attach(&db);
    let profile = first_run.service.create("Sasha").expect("a profile");

    // Delete the row without going through the service, the way a hand-edited
    // database or a future bulk operation would.
    first_run
        .context
        .profiles
        .delete(profile.id)
        .expect("deleting");

    let second_run = attach(&db);
    assert!(
        second_run
            .service
            .restore_active()
            .expect("restoring")
            .is_none(),
        "a stale pointer must not resolve to anything"
    );
    assert!(
        second_run
            .settings
            .app_get("active_profile_id")
            .expect("reading")
            .is_none(),
        "and it must not be left behind for the next start"
    );
}

#[test]
fn deleting_the_active_profile_leaves_none_active() {
    let (_db, harness) = fixture();

    let profile = harness.service.create("Sasha").expect("a profile");
    harness.service.delete(profile.id).expect("deleting");

    assert_eq!(harness.context.active_profile(), None);
    assert!(matches!(
        harness.context.require_active_profile(),
        Err(CoreError::NoActiveProfile)
    ));
    assert!(harness.service.list().expect("listing").is_empty());
}

#[test]
fn deleting_an_inactive_profile_leaves_the_active_one_alone() {
    let (_db, harness) = fixture();

    let active = harness.service.create("Sasha").expect("first");
    let other = harness.service.create("Kim").expect("second");

    harness.service.delete(other.id).expect("deleting");

    assert_eq!(harness.context.active_profile(), Some(active.id));
    assert_eq!(harness.service.list().expect("listing").len(), 1);
}

#[test]
fn profiles_are_listed_in_the_order_a_reader_expects() {
    let (_db, harness) = fixture();

    for name in ["Zoe", "andrei", "Kim"] {
        harness.service.create(name).expect("a profile");
    }

    let names: Vec<String> = harness
        .service
        .list()
        .expect("listing")
        .into_iter()
        .map(|profile| profile.name.to_string())
        .collect();

    assert_eq!(
        names,
        vec!["andrei", "Kim", "Zoe"],
        "alphabetical regardless of case"
    );
}

#[test]
fn renaming_keeps_the_creation_time_and_the_identity() {
    let (_db, harness) = fixture();

    let original = harness.service.create("Sasha").expect("a profile");
    harness.clock.advance_days(5);

    let renamed = harness
        .service
        .rename(original.id, "  Sasha K.  ")
        .expect("renaming");

    assert_eq!(renamed.id, original.id);
    assert_eq!(renamed.name.as_str(), "Sasha K.", "the name is trimmed");
    assert_eq!(
        renamed.created_at, original.created_at,
        "renaming is not re-creating"
    );
}

#[test]
fn history_and_theme_survive_a_round_trip() {
    let (_db, harness) = fixture();

    let profile = harness.service.create("Sasha").expect("a profile");
    harness
        .service
        .set_history_enabled(profile.id, true)
        .expect("enabling history");
    harness
        .service
        .set_theme(profile.id, ThemeMode::Light)
        .expect("switching theme");

    let stored = harness.service.get(profile.id).expect("reading it back");
    assert!(stored.history_enabled);
    assert_eq!(stored.theme, ThemeMode::Light);
    assert_eq!(
        stored.history_retention_days, 30,
        "the retention window is unchanged by turning history on"
    );
}

#[test]
fn settings_written_by_one_profile_are_invisible_to_another() {
    let (_db, harness) = fixture();

    let sasha = harness.service.create("Sasha").expect("first");
    let kim = harness.service.create("Kim").expect("second");
    let now = harness.clock.now();

    harness
        .settings
        .profile_set(sasha.id, "volume", &SettingValue::Float(0.4), now)
        .expect("writing");

    let stored = harness
        .settings
        .profile_get(sasha.id, "volume")
        .expect("reading")
        .expect("it is set")
        .as_float()
        .expect("a number");
    assert!((stored - 0.4).abs() < f64::EPSILON);

    assert!(
        harness
            .settings
            .profile_get(kim.id, "volume")
            .expect("reading")
            .is_none(),
        "one listener's settings are not another's"
    );
}

#[test]
fn a_setting_can_be_overwritten_and_removed() {
    let (_db, harness) = fixture();
    let now = harness.clock.now();

    harness
        .settings
        .app_set("last_screen", &SettingValue::from("library"), now)
        .expect("writing");
    harness
        .settings
        .app_set("last_screen", &SettingValue::from("radio"), now)
        .expect("overwriting");

    assert_eq!(
        harness
            .settings
            .app_get("last_screen")
            .expect("reading")
            .expect("it is set")
            .as_text()
            .expect("text"),
        "radio"
    );

    harness
        .settings
        .app_remove("last_screen")
        .expect("removing");
    assert!(
        harness
            .settings
            .app_get("last_screen")
            .expect("reading")
            .is_none()
    );
    harness
        .settings
        .app_remove("last_screen")
        .expect("removing something absent is not an error");
}

#[test]
fn library_folders_belong_to_their_profile_and_round_trip() {
    let (_db, harness) = fixture();

    let sasha = harness.service.create("Sasha").expect("first");
    let kim = harness.service.create("Kim").expect("second");

    let folder = ProfileFolder {
        id: ProfileFolderId::new(),
        profile_id: sasha.id,
        path: "D:/Music".into(),
        include_subfolders: true,
        enabled: true,
        last_scan_at: None,
    };
    harness.settings.save_folder(&folder).expect("saving");

    assert_eq!(
        harness.settings.list_folders(sasha.id).expect("listing"),
        vec![folder.clone()]
    );
    assert!(
        harness
            .settings
            .list_folders(kim.id)
            .expect("listing")
            .is_empty(),
        "folders are per-profile"
    );

    harness.settings.delete_folder(&folder).expect("removing");
    assert!(
        harness
            .settings
            .list_folders(sasha.id)
            .expect("listing")
            .is_empty()
    );
}

#[test]
fn deleting_a_profile_takes_its_settings_and_folders_with_it() {
    let (_db, harness) = fixture();

    let profile = harness.service.create("Sasha").expect("a profile");
    let now = harness.clock.now();

    harness
        .settings
        .profile_set(profile.id, "volume", &SettingValue::Float(0.4), now)
        .expect("writing a setting");
    harness
        .settings
        .save_folder(&ProfileFolder {
            id: ProfileFolderId::new(),
            profile_id: profile.id,
            path: "D:/Music".into(),
            include_subfolders: true,
            enabled: true,
            last_scan_at: None,
        })
        .expect("adding a folder");

    harness.service.delete(profile.id).expect("deleting");

    assert!(
        harness
            .settings
            .profile_get(profile.id, "volume")
            .expect("reading")
            .is_none()
    );
    assert!(
        harness
            .settings
            .list_folders(profile.id)
            .expect("listing")
            .is_empty()
    );
}

/// A chooser that always hands back the same file, so a test can press the
/// button without a dialog opening.
struct AlwaysPicks(std::path::PathBuf);

impl FolderPickerPort for AlwaysPicks {
    fn pick_folder(&self, _: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        Ok(None)
    }
    fn pick_image(&self, _: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        Ok(Some(self.0.clone()))
    }
    fn suggested_music_folder(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// A chooser somebody closed without choosing.
struct PicksNothing;

impl FolderPickerPort for PicksNothing {
    fn pick_folder(&self, _: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        Ok(None)
    }
    fn pick_image(&self, _: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
        Ok(None)
    }
    fn suggested_music_folder(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// The smallest thing the domain will accept as a picture: a PNG's own first
/// eight bytes. Nothing decodes it, and nothing here needs to.
const A_PNG: &[u8] = b"\x89PNG\r\n\x1a\n and then some bytes";

/// A service that can choose pictures, over the same database.
fn with_pictures(
    db: &TempDb,
    context: &Arc<AppContext>,
    picker: Arc<dyn FolderPickerPort>,
) -> (ProfileService, Arc<FileArtworkCache>) {
    let artwork =
        Arc::new(FileArtworkCache::new(db.directory().join("artwork")).expect("an artwork cache"));
    let service = ProfileService::with_covers(
        Arc::clone(context),
        CoverPorts {
            artwork: Arc::clone(&artwork) as _,
            picker,
            files: Arc::new(LocalFileSystem),
        },
    );
    (service, artwork)
}

#[test]
fn a_listener_can_put_a_picture_beside_their_name_and_take_it_off_again() {
    let db = TempDb::new();
    let harness = attach(&db);
    let profile = harness.service.create("Sasha").expect("a profile");

    let picture = db.directory().join("me.png");
    std::fs::write(&picture, A_PNG).expect("written");

    let (service, _artwork) = with_pictures(
        &db,
        &harness.context,
        Arc::new(AlwaysPicks(picture)) as Arc<dyn FolderPickerPort>,
    );

    assert!(service.avatar(profile.id).is_none(), "nobody has one yet");
    assert!(service.choose_avatar(profile.id).expect("chosen"));
    let path = service.avatar(profile.id).expect("a picture now");
    assert!(path.exists(), "and it is a file on disk");

    service.clear_avatar(profile.id).expect("taken off");
    assert!(
        service.avatar(profile.id).is_none(),
        "the initial stands there again"
    );
}

#[test]
fn closing_the_chooser_is_an_answer_rather_than_a_failure() {
    let db = TempDb::new();
    let harness = attach(&db);
    let profile = harness.service.create("Sasha").expect("a profile");

    let (service, _artwork) = with_pictures(
        &db,
        &harness.context,
        Arc::new(PicksNothing) as Arc<dyn FolderPickerPort>,
    );

    assert!(
        !service.choose_avatar(profile.id).expect("not an error"),
        "somebody changed their mind, which is not a thing to report"
    );
    assert!(service.avatar(profile.id).is_none());
}

#[test]
fn a_file_that_is_not_a_picture_is_refused_by_name() {
    let db = TempDb::new();
    let harness = attach(&db);
    let profile = harness.service.create("Sasha").expect("a profile");

    let not_a_picture = db.directory().join("notes.txt");
    std::fs::write(&not_a_picture, b"these are not pixels").expect("written");

    let (service, _artwork) = with_pictures(
        &db,
        &harness.context,
        Arc::new(AlwaysPicks(not_a_picture)) as Arc<dyn FolderPickerPort>,
    );

    let refused = service.choose_avatar(profile.id).expect_err("refused");
    assert!(
        refused.to_string().contains("notes.txt"),
        "the file is named, because the listener chose it: {refused}"
    );
}

#[test]
fn deleting_a_profile_takes_its_picture_with_it() {
    let db = TempDb::new();
    let harness = attach(&db);
    let profile = harness.service.create("Sasha").expect("a profile");

    let picture = db.directory().join("me.png");
    std::fs::write(&picture, A_PNG).expect("written");
    let (service, artwork) = with_pictures(
        &db,
        &harness.context,
        Arc::new(AlwaysPicks(picture)) as Arc<dyn FolderPickerPort>,
    );
    service.choose_avatar(profile.id).expect("chosen");

    let stored = service.avatar(profile.id).expect("a picture");
    service.delete(profile.id).expect("deleted");

    assert!(
        !stored.exists(),
        "a portrait left on the disk is the one piece of a deleted listener \
         that would still be there"
    );
    assert!(artwork.path_for(CoverOf::Profile(profile.id)).is_none());
}
