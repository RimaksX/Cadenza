//! The saved queue survives a restart.
//!
//! Against the real schema, because the foreign keys and the lane ordering are
//! the whole of what this adapter has to get right, and neither exists in a
//! fake.

use std::path::PathBuf;
use std::sync::Arc;

use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, PlaylistId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::{MediaFileRepositoryPort, QueueRepositoryPort};
use cadenza_core::domain::queue::{Queue, QueueEntry, QueueOrigin, RepeatMode};
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteMediaFileRepository, SqliteProfileRepository, SqliteQueueRepository,
    SqliteSettingsRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

/// A profile and three catalogued files to queue.
struct Harness {
    queues: SqliteQueueRepository,
    profile_id: ProfileId,
    files: Vec<MediaFileId>,
    /// Declared last on purpose: fields are dropped in declaration order, and
    /// the fixture cannot delete its directory while anything above it still
    /// holds a connection to the database inside it.
    _db: TempDb,
}

fn harness() -> Harness {
    let db = TempDb::new();

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    let profile = ProfileService::new(context)
        .create("Sasha")
        .expect("a profile");

    let media_files = SqliteMediaFileRepository::new(db.pool().clone());
    let files = (0..3)
        .map(|index| {
            let media_file = catalogued(index);
            media_files.save(&media_file).expect("catalogued");
            media_file.id
        })
        .collect();

    Harness {
        queues: SqliteQueueRepository::new(db.pool().clone()),
        _db: db,
        profile_id: profile.id,
        files,
    }
}

fn catalogued(index: u8) -> MediaFile {
    MediaFile {
        id: MediaFileId::new(),
        path: PathBuf::from(format!("C:/music/track-{index}.flac")),
        file_hash: None,
        file_size: 1_024,
        file_mtime: Timestamp::from_millis(0),
        format: AudioFormat::Flac,
        properties: AudioProperties {
            duration: DurationMs::from_secs(200),
            sample_rate: 44_100,
            channels: 2,
            bitrate: None,
        },
        metadata_version: None,
        metadata_extracted_at: None,
        state: FileState::Available,
        created_at: Timestamp::from_millis(0),
        updated_at: Timestamp::from_millis(0),
    }
}

/// A queue with something in every lane.
fn filled(harness: &Harness) -> Queue {
    let mut queue = Queue::new(harness.profile_id);
    queue.repeat = RepeatMode::All;
    queue.shuffle = true;
    queue.current = Some(QueueEntry {
        media_file_id: harness.files[0],
        origin: QueueOrigin::Library,
    });
    queue.manual.push_back(QueueEntry {
        media_file_id: harness.files[1],
        origin: QueueOrigin::Playlist(PlaylistId::new()),
    });
    queue.upcoming.push_back(QueueEntry {
        media_file_id: harness.files[2],
        origin: QueueOrigin::Library,
    });
    queue.history.push(QueueEntry {
        media_file_id: harness.files[1],
        origin: QueueOrigin::Library,
    });
    queue
}

#[test]
fn a_queue_comes_back_exactly_as_it_was_left() {
    let harness = harness();
    let saved = filled(&harness);
    harness.queues.save(&saved).expect("saved");

    let restored = harness
        .queues
        .load(harness.profile_id)
        .expect("read")
        .expect("a saved queue");

    assert_eq!(restored, saved, "every lane, its order, and both modes");
}

#[test]
fn saving_again_replaces_rather_than_appends() {
    let harness = harness();
    harness.queues.save(&filled(&harness)).expect("saved");

    let mut second = Queue::new(harness.profile_id);
    second.current = Some(QueueEntry {
        media_file_id: harness.files[2],
        origin: QueueOrigin::Library,
    });
    harness.queues.save(&second).expect("saved again");

    let restored = harness
        .queues
        .load(harness.profile_id)
        .expect("read")
        .expect("a saved queue");

    assert_eq!(restored, second);
    assert!(restored.manual.is_empty(), "the old lanes are gone");
    assert!(restored.history.is_empty());
}

#[test]
fn a_profile_that_has_never_played_has_no_queue() {
    let harness = harness();
    assert!(
        harness
            .queues
            .load(harness.profile_id)
            .expect("read")
            .is_none(),
        "no row is not an empty queue: the difference is whether to restore"
    );
}

#[test]
fn clearing_takes_the_entries_with_it() {
    let harness = harness();
    harness.queues.save(&filled(&harness)).expect("saved");
    harness.queues.clear(harness.profile_id).expect("cleared");

    assert!(
        harness
            .queues
            .load(harness.profile_id)
            .expect("read")
            .is_none()
    );

    // And the entries went with the state row rather than being orphaned by it.
    harness
        .queues
        .save(&Queue::new(harness.profile_id))
        .expect("saved");
    let restored = harness
        .queues
        .load(harness.profile_id)
        .expect("read")
        .expect("a saved queue");
    assert!(restored.is_empty());
}
