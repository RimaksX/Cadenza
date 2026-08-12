//! Playlists work: the last third of the M7 definition of done.
//!
//! The real service over the real adapter against a real database, because what
//! has to be right here is the ordering of rows in a table and what survives a
//! restart — neither of which a fake repository would test.

use std::path::PathBuf;
use std::sync::Arc;

use cadenza_core::CoreError;
use cadenza_core::application::services::{PlaylistPorts, PlaylistService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::{MediaFileRepositoryPort, TrackRepositoryPort};
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteMediaFileRepository, SqlitePlaylistRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_testkit::{TempDb, TestClock};

/// A profile with a four-track library and a playlist service over it.
struct Harness {
    db: TempDb,
    playlists: PlaylistService,
    profile_id: ProfileId,
    tracks: Vec<MediaFileId>,
}

fn harness() -> Harness {
    let db = TempDb::new();

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");
    context.set_active_profile(profile.id);

    let media_files = SqliteMediaFileRepository::new(db.pool().clone());
    let track_repo = SqliteTrackRepository::new(db.pool().clone());

    let tracks = ["one", "two", "three", "four"]
        .into_iter()
        .map(|name| {
            let media_file = catalogued(name);
            media_files.save(&media_file).expect("catalogued");
            track_repo
                .save(&in_library(profile.id, media_file.id, name))
                .expect("in the library");
            media_file.id
        })
        .collect();

    Harness {
        playlists: service(&db, profile.id),
        db,
        profile_id: profile.id,
        tracks,
    }
}

/// A playlist service as one run of the application builds it.
fn service(db: &TempDb, profile_id: ProfileId) -> PlaylistService {
    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    context.set_active_profile(profile_id);

    PlaylistService::new(
        context,
        PlaylistPorts {
            playlists: Arc::new(SqlitePlaylistRepository::new(db.pool().clone())),
            tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
        },
    )
}

fn catalogued(name: &str) -> MediaFile {
    MediaFile {
        id: MediaFileId::new(),
        path: PathBuf::from(format!("C:/music/{name}.flac")),
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

fn in_library(profile_id: ProfileId, media_file_id: MediaFileId, title: &str) -> Track {
    Track {
        profile_id,
        media_file_id,
        title: title.to_owned(),
        artist_id: None,
        album_id: None,
        track_no: None,
        disc_no: None,
        year: None,
        added_at: Timestamp::from_millis(0),
        removed_at: None,
    }
}

impl Harness {
    /// The titles in a playlist, in its order.
    fn titles(&self, id: cadenza_core::domain::ids::PlaylistId) -> Vec<String> {
        self.playlists
            .tracks_of(id)
            .expect("a playlist")
            .into_iter()
            .map(|summary| summary.title)
            .collect()
    }
}

#[test]
fn a_playlist_keeps_the_order_tracks_were_added_in() {
    let harness = harness();
    let playlist = harness.playlists.create("Late night").expect("created");

    // Deliberately not library order, which is alphabetical: a playlist is the
    // listener's order, not the library's.
    for index in [3, 0, 2] {
        harness
            .playlists
            .add_track(playlist.id, harness.tracks[index])
            .expect("added");
    }

    assert_eq!(harness.titles(playlist.id), ["four", "one", "three"]);
    assert_eq!(
        harness.playlists.list().expect("listed")[0].track_count,
        3,
        "the listing knows how much is in it"
    );
}

#[test]
fn the_same_track_can_appear_twice() {
    let harness = harness();
    let playlist = harness.playlists.create("Bookends").expect("created");

    harness
        .playlists
        .add_track(playlist.id, harness.tracks[0])
        .expect("added");
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[1])
        .expect("added");
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[0])
        .expect("added again");

    assert_eq!(
        harness.titles(playlist.id),
        ["one", "two", "one"],
        "an entry has its own identity, so both copies stand"
    );
}

#[test]
fn removing_an_entry_closes_the_gap() {
    let harness = harness();
    let playlist = harness.playlists.create("Set").expect("created");
    for index in 0..4 {
        harness
            .playlists
            .add_track(playlist.id, harness.tracks[index])
            .expect("added");
    }

    harness
        .playlists
        .remove_at(playlist.id, 1)
        .expect("removed");

    assert_eq!(harness.titles(playlist.id), ["one", "three", "four"]);
    // Positions are renumbered rather than left with a hole, which is what the
    // next insert depends on.
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[1])
        .expect("added");
    assert_eq!(harness.titles(playlist.id), ["one", "three", "four", "two"]);
}

#[test]
fn an_entry_can_be_moved_to_any_position() {
    let harness = harness();
    let playlist = harness.playlists.create("Set").expect("created");
    for index in 0..4 {
        harness
            .playlists
            .add_track(playlist.id, harness.tracks[index])
            .expect("added");
    }

    // First to last, which is the reorder most likely to renumber every row.
    harness
        .playlists
        .move_entry(playlist.id, 0, 3)
        .expect("moved");
    assert_eq!(harness.titles(playlist.id), ["two", "three", "four", "one"]);

    harness
        .playlists
        .move_entry(playlist.id, 3, 0)
        .expect("moved back");
    assert_eq!(harness.titles(playlist.id), ["one", "two", "three", "four"]);

    // Dropped past the end means the end, the way dragging a row to the bottom
    // of a list does.
    harness
        .playlists
        .move_entry(playlist.id, 0, 99)
        .expect("moved");
    assert_eq!(harness.titles(playlist.id), ["two", "three", "four", "one"]);
}

#[test]
fn a_playlist_survives_a_restart() {
    let harness = harness();
    let playlist = harness.playlists.create("Late night").expect("created");
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[2])
        .expect("added");
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[0])
        .expect("added");

    let restarted = service(&harness.db, harness.profile_id);
    let listed = restarted.list().expect("listed");

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].playlist.name, "Late night");
    assert_eq!(
        restarted
            .tracks_of(playlist.id)
            .expect("a playlist")
            .into_iter()
            .map(|summary| summary.title)
            .collect::<Vec<_>>(),
        ["three", "one"]
    );
}

#[test]
fn names_are_unique_within_a_profile() {
    let harness = harness();
    harness.playlists.create("Late night").expect("created");

    let message = harness
        .playlists
        .create("  late NIGHT  ")
        .expect_err("a second one")
        .to_string();
    assert!(
        message.contains("already exists"),
        "the message names the problem, not the constraint: {message}"
    );
}

#[test]
fn a_playlist_can_be_renamed_and_deleted_without_touching_its_tracks() {
    let harness = harness();
    let playlist = harness.playlists.create("Late night").expect("created");
    harness
        .playlists
        .add_track(playlist.id, harness.tracks[0])
        .expect("added");

    let renamed = harness
        .playlists
        .rename(playlist.id, "Early morning")
        .expect("renamed");
    assert_eq!(renamed.name, "Early morning");

    harness.playlists.delete(playlist.id).expect("deleted");
    assert!(harness.playlists.list().expect("listed").is_empty());

    // The library is untouched: deleting a list of tracks is not deleting the
    // tracks.
    let tracks = SqliteTrackRepository::new(harness.db.pool().clone())
        .summaries_for_profile(harness.profile_id)
        .expect("the library");
    assert_eq!(tracks.len(), 4);
}

#[test]
fn another_profiles_playlist_does_not_exist() {
    let harness = harness();
    let playlist = harness.playlists.create("Late night").expect("created");

    // A second profile over the same database, as a switch would give.
    let other = ProfileService::new(Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(harness.db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(harness.db.pool().clone())),
    )))
    .create("Alex")
    .expect("a second profile");

    let theirs = service(&harness.db, other.id);

    assert!(theirs.list().expect("listed").is_empty());
    assert!(
        matches!(
            theirs.get(playlist.id),
            Err(CoreError::NotFound {
                entity: "playlist",
                ..
            })
        ),
        "playlists are never shared, so somebody else's is simply not there"
    );
}

#[test]
fn a_track_the_profile_does_not_have_cannot_be_added() {
    let harness = harness();
    let playlist = harness.playlists.create("Late night").expect("created");

    assert!(
        harness
            .playlists
            .add_track(playlist.id, MediaFileId::new())
            .is_err(),
        "a playlist entry has to stand for something the profile can play"
    );
}
