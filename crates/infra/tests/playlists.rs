//! Playlists work.
//!
//! The real service over the real adapter against a real database, because what
//! has to be right here is the ordering of rows in a table and what survives a
//! restart — neither of which a fake repository would test.

use std::path::PathBuf;
use std::sync::Arc;

use cadenza_core::CoreError;
use cadenza_core::application::services::{PlaylistPorts, PlaylistService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::{MediaFileId, PlayEventId, ProfileId};
use cadenza_core::domain::media_file::{AudioFormat, AudioProperties, FileState, MediaFile};
use cadenza_core::domain::ports::repositories::{
    MediaFileRepositoryPort, PlayEventRepositoryPort, TrackRepositoryPort,
};
use cadenza_core::domain::stats::{PlayEvent, PlayOutcome, PlaySource};
use cadenza_core::domain::track::Track;
use cadenza_core::domain::value_objects::{DurationMs, Timestamp};
use cadenza_infra::db::repositories::{
    SqliteHistoryRepository, SqliteMediaFileRepository, SqlitePlaylistRepository,
    SqliteProfileRepository, SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_infra::library::LocalFileSystem;
use cadenza_infra::metadata::FileArtworkCache;
use cadenza_testkit::{TempDb, TestClock, test_clock::DEFAULT_START};

/// A profile with a four-track library and a playlist service over it.
struct Harness {
    playlists: PlaylistService,
    profile_id: ProfileId,
    tracks: Vec<MediaFileId>,
    /// Declared last on purpose: fields are dropped in declaration order, and
    /// the fixture cannot delete its directory while anything above it still
    /// holds a connection to the database inside it.
    db: TempDb,
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
            artwork: Arc::new(
                FileArtworkCache::new(db.directory().join("artwork")).expect("an artwork cache"),
            ),
            picker: Arc::new(NoPicker),
            files: Arc::new(LocalFileSystem),
            stats: Arc::new(SqliteHistoryRepository::new(db.pool().clone())),
        },
    )
}

/// A chooser nobody opens: no test here chooses a cover.
struct NoPicker;

impl cadenza_core::domain::ports::folder_picker::FolderPickerPort for NoPicker {
    fn pick_folder(&self, _title: &str) -> cadenza_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn pick_image(&self, _title: &str) -> cadenza_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn suggested_music_folder(&self) -> Option<PathBuf> {
        None
    }
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

impl Harness {
    /// Writes down `times` finished listens of a track.
    ///
    /// Through the real repository, because what the favourites list counts is
    /// what `top_tracks` counts, and a test that inserted its own rows would be
    /// asserting against its own idea of the schema.
    fn played(&self, media_file_id: MediaFileId, times: u32) {
        let history = SqliteHistoryRepository::new(self.db.pool().clone());
        for index in 0..times {
            history
                .append(&PlayEvent {
                    id: PlayEventId::new(),
                    profile_id: self.profile_id,
                    media_file_id,
                    source: PlaySource::Library,
                    // Inside the retained window, which is what the count
                    // covers: an event stamped at the epoch is a month of
                    // listening the history has already forgotten.
                    started_at: Timestamp::from_millis(
                        DEFAULT_START.as_millis() - 1_000 + i64::from(index),
                    ),
                    ended_at: Some(Timestamp::from_millis(DEFAULT_START.as_millis())),
                    played: DurationMs::from_secs(180),
                    duration: DurationMs::from_secs(200),
                    outcome: PlayOutcome::Completed,
                })
                .expect("recorded");
        }
    }

    /// Skipped listens, which are listens and are not favourites.
    fn skipped(&self, media_file_id: MediaFileId, times: u32) {
        let history = SqliteHistoryRepository::new(self.db.pool().clone());
        for index in 0..times {
            history
                .append(&PlayEvent {
                    id: PlayEventId::new(),
                    profile_id: self.profile_id,
                    media_file_id,
                    source: PlaySource::Library,
                    started_at: Timestamp::from_millis(
                        DEFAULT_START.as_millis() - 1_000 + i64::from(index),
                    ),
                    ended_at: Some(Timestamp::from_millis(DEFAULT_START.as_millis())),
                    played: DurationMs::from_secs(2),
                    duration: DurationMs::from_secs(200),
                    outcome: PlayOutcome::Skipped,
                })
                .expect("recorded");
        }
    }

    /// The titles in the favourites list, in the order it holds them.
    fn favourites(&self) -> Vec<String> {
        let list = self.playlists.favourites().expect("a favourites list");
        self.playlists
            .tracks_of(list.id)
            .expect("its tracks")
            .into_iter()
            .map(|summary| summary.title)
            .collect()
    }
}

#[test]
fn the_favourites_list_is_there_without_anybody_making_it() {
    let harness = harness();

    let list = harness.playlists.favourites().expect("a favourites list");
    assert_eq!(list.name, "Favourites");
    assert!(list.is_favourites());
    assert!(
        !list.is_manually_ordered(),
        "its order is the play count's, so nothing may be dragged in it"
    );

    // Asking twice is asking about the same list, not making a second one.
    let again = harness.playlists.favourites().expect("the same list");
    assert_eq!(again.id, list.id);
    assert_eq!(
        harness.playlists.list().expect("a listing").len(),
        1,
        "one list, however many times it was asked for"
    );
}

#[test]
fn the_favourites_list_cannot_be_renamed_or_deleted() {
    let harness = harness();
    let list = harness.playlists.favourites().expect("a favourites list");

    assert!(
        harness.playlists.rename(list.id, "Rubbish").is_err(),
        "it is the list a listener gets back to"
    );
    assert!(harness.playlists.delete(list.id).is_err());
    assert_eq!(
        harness.playlists.get(list.id).expect("still there").name,
        "Favourites"
    );
}

#[test]
fn a_track_played_twice_is_a_favourite_and_one_played_once_is_not() {
    let harness = harness();

    harness.played(harness.tracks[0], 5);
    harness.played(harness.tracks[1], 2);
    harness.played(harness.tracks[2], 1);
    harness.skipped(harness.tracks[3], 9);

    harness.playlists.refresh_favourites().expect("rebuilt");

    assert_eq!(
        harness.favourites(),
        vec!["one", "two"],
        "most played first; once is not coming back to it, and nine skips are          not a favourite however many there are"
    );
}

#[test]
fn what_was_pinned_stays_when_the_counts_move() {
    let harness = harness();
    let list = harness.playlists.favourites().expect("a favourites list");

    // Pinned by hand, and never played.
    harness
        .playlists
        .add_track(list.id, harness.tracks[3])
        .expect("pinned");
    harness.played(harness.tracks[0], 4);
    harness.playlists.refresh_favourites().expect("rebuilt");

    assert_eq!(
        harness.favourites(),
        vec!["four", "one"],
        "what was pinned is on top, and what is played follows it"
    );

    // A second rebuild is the same answer, not a growing list.
    harness
        .playlists
        .refresh_favourites()
        .expect("rebuilt again");
    assert_eq!(harness.favourites(), vec!["four", "one"]);
}

#[test]
fn pinning_a_track_that_is_already_there_makes_it_stay() {
    let harness = harness();
    let list = harness.playlists.favourites().expect("a favourites list");

    harness.played(harness.tracks[0], 3);
    harness.playlists.refresh_favourites().expect("rebuilt");
    assert_eq!(harness.favourites(), vec!["one"]);

    // The listener says so themselves. Nothing visible changes - and then the
    // history goes, as a thirty-day history does, and the difference shows.
    harness
        .playlists
        .add_track(list.id, harness.tracks[0])
        .expect("pinned");
    assert_eq!(harness.favourites(), vec!["one"], "still one row, not two");

    SqliteHistoryRepository::new(harness.db.pool().clone())
        .purge_all(harness.profile_id)
        .expect("a month went by");
    harness.playlists.refresh_favourites().expect("rebuilt");

    assert_eq!(
        harness.favourites(),
        vec!["one"],
        "a pinned track outlives the count that first put it there"
    );
}

#[test]
fn a_track_that_is_there_because_it_is_played_cannot_be_taken_out() {
    let harness = harness();
    let list = harness.playlists.favourites().expect("a favourites list");

    harness.played(harness.tracks[0], 4);
    harness
        .playlists
        .add_track(list.id, harness.tracks[3])
        .expect("pinned");
    harness.playlists.refresh_favourites().expect("rebuilt");
    assert_eq!(harness.favourites(), vec!["four", "one"]);

    // Row 1 is there because of the count, and removing it would only bring it
    // back the next time a track ended.
    assert!(
        harness.playlists.remove_at(list.id, 1).is_err(),
        "there is nothing this could mean that would last"
    );

    // Row 0 was pinned by hand, and unpinning it is a thing a listener can do.
    harness.playlists.remove_at(list.id, 0).expect("unpinned");
    assert_eq!(harness.favourites(), vec!["one"]);
}

#[test]
fn a_listener_who_already_has_a_list_called_favourites_keeps_it() {
    let harness = harness();
    let mine = harness.playlists.create("Favourites").expect("my own list");

    let automatic = harness.playlists.favourites().expect("a favourites list");

    assert_ne!(automatic.id, mine.id);
    assert_eq!(automatic.name, "Favourites 2");
    assert_eq!(
        harness.playlists.get(mine.id).expect("still mine").name,
        "Favourites",
        "and theirs is untouched"
    );
}

#[test]
fn a_favourite_whose_file_has_left_the_library_leaves_with_it() {
    let harness = harness();
    harness.played(harness.tracks[0], 4);
    harness.played(harness.tracks[1], 3);
    harness.playlists.refresh_favourites().expect("rebuilt");
    assert_eq!(harness.favourites(), vec!["one", "two"]);

    // Removed from the library the way the library removes things.
    let tracks = SqliteTrackRepository::new(harness.db.pool().clone());
    tracks
        .remove(
            harness.profile_id,
            harness.tracks[0],
            Timestamp::from_millis(DEFAULT_START.as_millis()),
        )
        .expect("gone from the library");

    harness.playlists.refresh_favourites().expect("rebuilt");
    assert_eq!(
        harness.favourites(),
        vec!["two"],
        "there is nothing left to play, so it is not a favourite"
    );
}

#[test]
fn the_heart_is_lit_for_a_track_that_is_in_the_favourites_either_way() {
    let harness = harness();

    assert!(
        !harness
            .playlists
            .is_favourite(harness.tracks[0])
            .expect("asked"),
        "nothing is a favourite yet"
    );

    // One arrives by being pressed, the other by being played.
    harness
        .playlists
        .set_favourite(harness.tracks[0], true)
        .expect("pinned");
    harness.played(harness.tracks[1], 4);
    harness.playlists.refresh_favourites().expect("rebuilt");

    for track in [harness.tracks[0], harness.tracks[1]] {
        assert!(
            harness.playlists.is_favourite(track).expect("asked"),
            "the heart says what is in the list, not how it got there"
        );
    }
    assert!(
        !harness
            .playlists
            .is_favourite(harness.tracks[2])
            .expect("asked")
    );
}

#[test]
fn pressing_the_heart_twice_leaves_things_where_they_were() {
    let harness = harness();

    harness
        .playlists
        .set_favourite(harness.tracks[0], true)
        .expect("pinned");
    assert_eq!(harness.favourites(), vec!["one"]);

    harness
        .playlists
        .set_favourite(harness.tracks[0], false)
        .expect("unpinned");
    assert!(
        harness.favourites().is_empty(),
        "and it is out again: a switch that cannot be switched back is a trap"
    );
}

#[test]
fn the_heart_will_not_pretend_to_remove_what_the_count_put_there() {
    let harness = harness();
    harness.played(harness.tracks[0], 4);
    harness.playlists.refresh_favourites().expect("rebuilt");

    let refused = harness.playlists.set_favourite(harness.tracks[0], false);
    assert!(
        refused.is_err(),
        "it would come back the next time a track ended"
    );
    assert_eq!(harness.favourites(), vec!["one"]);

    // Nothing to undo is not a failure, though: a track that was never in the
    // list is already in the state the press was asking for.
    harness
        .playlists
        .set_favourite(harness.tracks[3], false)
        .expect("already out");
}

#[test]
fn unpinning_a_track_the_count_also_earns_leaves_it_in_the_list() {
    let harness = harness();
    harness.played(harness.tracks[0], 4);
    harness.playlists.refresh_favourites().expect("rebuilt");

    // Pinned on top of being earned, then unpinned. The count still holds it.
    harness
        .playlists
        .set_favourite(harness.tracks[0], true)
        .expect("pinned");
    harness
        .playlists
        .set_favourite(harness.tracks[0], false)
        .expect("unpinned");

    assert_eq!(
        harness.favourites(),
        vec!["one"],
        "the pin came off; what the listener keeps playing did not"
    );
    assert!(
        harness
            .playlists
            .is_favourite(harness.tracks[0])
            .expect("asked"),
        "so the heart stays lit, which is the truth about the list"
    );
}
