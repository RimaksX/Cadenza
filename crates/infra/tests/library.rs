//! Integration tests for scanning and import: the M4 definition of done.
//!
//! Real files on a real disk, read by the real tag reader, into a real database.
//! A scanner tested against a fake filesystem proves the branching and nothing
//! about whether lofty can actually read what was written.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cadenza_core::application::services::{LibraryPorts, LibraryService, ScanReport};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ids::ProfileId;
use cadenza_core::domain::media_file::{AudioFormat, FileState};
use cadenza_core::domain::ports::file_watcher::FileChange;
use cadenza_core::domain::ports::repositories::MediaFileRepositoryPort;
use cadenza_core::domain::review::{ReviewReason, ReviewResolution};
use cadenza_infra::db::repositories::{
    SqliteAlbumRepository, SqliteArtistRepository, SqliteGenreRepository,
    SqliteImportReviewRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_infra::library::LocalFileSystem;
use cadenza_infra::metadata::{FileArtworkCache, LoftyMetadataReader};
use cadenza_testkit::audio_fixtures::write_wav;
use cadenza_testkit::{TempDb, TestClock};
use lofty::config::WriteOptions;
use lofty::tag::{Accessor, Tag, TagExt, TagType};

/// A profile, a music folder and a wired library service.
struct Harness {
    music: PathBuf,
    library: LibraryService,
    media_files: Arc<SqliteMediaFileRepository>,
    profiles: ProfileService,
    profile_id: ProfileId,
    /// Declared last on purpose: fields are dropped in declaration order, and
    /// the fixture cannot delete its directory while anything above it still
    /// holds a connection to the database inside it.
    _db: TempDb,
}

fn harness(tag: &str) -> Harness {
    let db = TempDb::new();
    let root = db.directory().join(tag);
    let music = root.join("music");
    let cache = root.join("artwork");
    std::fs::create_dir_all(&music).expect("a music folder");

    let clock = Arc::new(TestClock::default());
    let events = Arc::new(InProcessEventBus::new());
    let profiles = Arc::new(SqliteProfileRepository::new(db.pool().clone()));
    let settings = Arc::new(SqliteSettingsRepository::new(db.pool().clone()));
    let media_files = Arc::new(SqliteMediaFileRepository::new(db.pool().clone()));

    let context = Arc::new(AppContext::new(clock, events, profiles, settings));
    let profile = ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");

    /// A chooser nobody opens: these tests hand the service paths directly.
    struct NoPicker;
    impl cadenza_core::domain::ports::folder_picker::FolderPickerPort for NoPicker {
        fn pick_folder(&self, _title: &str) -> cadenza_core::Result<Option<std::path::PathBuf>> {
            Ok(None)
        }
        fn suggested_music_folder(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    let ports = LibraryPorts {
        picker: Arc::new(NoPicker),
        files: Arc::new(LocalFileSystem),
        metadata: Arc::new(LoftyMetadataReader),
        artwork: Arc::new(FileArtworkCache::new(cache).expect("an artwork cache")),
        media_files: Arc::clone(&media_files) as _,
        tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
        artists: Arc::new(SqliteArtistRepository::new(db.pool().clone())),
        albums: Arc::new(SqliteAlbumRepository::new(db.pool().clone())),
        genres: Arc::new(SqliteGenreRepository::new(db.pool().clone())),
        reviews: Arc::new(SqliteImportReviewRepository::new(db.pool().clone())),
    };

    Harness {
        library: LibraryService::new(Arc::clone(&context), ports),
        music,
        media_files,
        profiles: ProfileService::new(context),
        profile_id: profile.id,
        _db: db,
    }
}

impl Harness {
    /// Adds the music folder once, then scans it.
    fn scan(&self, include_subfolders: bool) -> ScanReport {
        let folders = self.library.folders().expect("listing folders");
        let folder = match folders.first() {
            Some(existing) => existing.clone(),
            None => self
                .library
                .add_folder(&self.music, include_subfolders)
                .expect("adding the folder"),
        };
        self.library.scan_folder(&folder).expect("scanning")
    }

    fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = self
            .library
            .tracks()
            .expect("listing tracks")
            .into_iter()
            .map(|track| track.title)
            .collect();
        titles.sort();
        titles
    }
}

/// Writes ID3 tags into a file lofty can already read.
fn tag_file(path: &Path, title: &str, artist: &str, album: &str, genre: &str) {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(title.to_owned());
    tag.set_artist(artist.to_owned());
    tag.set_album(album.to_owned());
    tag.set_genre(genre.to_owned());
    tag.save_to_path(path, WriteOptions::default())
        .expect("writing tags");
}

#[test]
fn a_folder_of_music_becomes_a_library() {
    let harness = harness("basic");
    write_wav(&harness.music, "one.wav", 1, 10);
    write_wav(&harness.music, "two.wav", 1, 20);

    let report = harness.scan(true);

    assert_eq!(report.seen, 2);
    assert_eq!(report.added, 2);
    assert_eq!(report.failed, 0, "generated WAVs must be readable");
    assert_eq!(harness.titles(), vec!["one", "two"]);
}

#[test]
fn the_stream_properties_come_from_the_file_not_the_extension() {
    let harness = harness("properties");
    write_wav(&harness.music, "one.wav", 2, 10);
    harness.scan(true);

    let path = harness.music.join("one.wav");
    let file = harness
        .media_files
        .find_by_path(&path)
        .expect("looking it up")
        .expect("it was catalogued");

    assert_eq!(file.format, AudioFormat::Wav);
    assert_eq!(file.properties.sample_rate, 44_100);
    assert_eq!(file.properties.channels, 2);
    assert!(
        file.properties.duration.as_secs() >= 1,
        "two seconds of audio should not read as zero, got {}",
        file.properties.duration
    );
    assert!(
        file.file_hash.is_some(),
        "hashing feeds duplicate detection"
    );
}

#[test]
fn a_second_scan_finds_nothing_new() {
    let harness = harness("rescan");
    write_wav(&harness.music, "one.wav", 1, 10);

    assert_eq!(harness.scan(true).added, 1);

    let second = harness.scan(true);
    assert_eq!(second.added, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(
        second.unchanged, 1,
        "an unchanged file must not be re-read or re-hashed"
    );
}

#[test]
fn a_copy_of_a_track_is_held_back_for_a_decision() {
    let harness = harness("duplicate");
    // Same fill, so the bytes are identical: this is what a duplicate is.
    write_wav(&harness.music, "original.wav", 1, 33);
    write_wav(&harness.music.join("copies"), "same.wav", 1, 33);

    let report = harness.scan(true);

    assert_eq!(report.seen, 2);
    assert_eq!(report.added, 1);
    assert_eq!(
        report.duplicates, 1,
        "the second copy is not imported silently"
    );
    assert_eq!(
        harness.titles(),
        vec!["original"],
        "only one of them reaches the library"
    );

    let pending = harness.library.pending_reviews().expect("the review queue");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].reason, ReviewReason::Duplicate);
    assert!(
        pending[0].duplicate_media_file_id.is_some(),
        "the entry must say what it duplicates"
    );
}

#[test]
fn rescanning_does_not_stack_up_review_entries() {
    let harness = harness("review-once");
    write_wav(&harness.music, "original.wav", 1, 33);
    write_wav(&harness.music, "copy.wav", 1, 33);

    harness.scan(true);
    harness.scan(true);

    assert_eq!(
        harness.library.pending_reviews().expect("the queue").len(),
        1,
        "the same unresolved problem must not be raised twice"
    );
}

#[test]
fn a_rescan_does_not_quietly_import_a_file_awaiting_a_decision() {
    // Found by running the binary: on the second scan the duplicate took the
    // unchanged path, which found no track row and added one — importing the
    // very file the review queue was holding back.
    let harness = harness("held-back-stays-back");
    write_wav(&harness.music, "a_first.wav", 1, 33);
    write_wav(&harness.music, "b_second.wav", 1, 33);

    assert_eq!(harness.scan(true).added, 1);

    let second = harness.scan(true);
    assert_eq!(
        second.added, 0,
        "nothing new should be added by a scan that changed nothing"
    );
    assert_eq!(second.duplicates, 1, "it is still held back");
    assert_eq!(
        harness.titles(),
        vec!["a first"],
        "the undecided copy must stay out of the library"
    );
}

#[test]
fn choosing_to_add_a_duplicate_anyway_puts_it_in_the_library() {
    let harness = harness("add-anyway");
    // A folder is walked in sorted order, so the first name is the one that
    // reaches the library and the second is the one held back.
    write_wav(&harness.music, "a_first.wav", 1, 33);
    write_wav(&harness.music, "b_second.wav", 1, 33);
    harness.scan(true);

    let pending = harness.library.pending_reviews().expect("the queue");
    harness
        .library
        .resolve_review(pending[0].id, ReviewResolution::AddAnyway)
        .expect("resolving");

    assert_eq!(harness.titles(), vec!["a first", "b second"]);
    assert!(
        harness
            .library
            .pending_reviews()
            .expect("the queue")
            .is_empty(),
        "a resolved entry leaves the queue"
    );
}

#[test]
fn choosing_to_keep_the_existing_track_leaves_the_library_alone() {
    let harness = harness("keep-existing");
    write_wav(&harness.music, "a_first.wav", 1, 33);
    write_wav(&harness.music, "b_second.wav", 1, 33);
    harness.scan(true);

    let pending = harness.library.pending_reviews().expect("the queue");
    harness
        .library
        .resolve_review(pending[0].id, ReviewResolution::KeepExisting)
        .expect("resolving");

    assert_eq!(
        harness.titles(),
        vec!["a first"],
        "declining the copy leaves the library as it was"
    );
    assert!(
        harness
            .library
            .pending_reviews()
            .expect("the queue")
            .is_empty()
    );
}

#[test]
fn a_vanished_copy_does_not_hold_back_a_new_file() {
    // Found by running the binary: the catalogue is global and outlives the
    // profiles that used it, so it fills up with rows for files that have since
    // been deleted. Blocking an import as a duplicate of one of those leaves the
    // listener with a decision they cannot act on and an empty library.
    let harness = harness("vanished");
    let original = write_wav(&harness.music, "gone.wav", 1, 77);
    harness.scan(true);
    assert_eq!(harness.titles(), vec!["gone"]);

    // The file leaves, its catalogue row stays.
    std::fs::remove_file(&original).expect("removing the original");

    // The same content turns up somewhere else.
    write_wav(&harness.music, "found_again.wav", 1, 77);
    let report = harness.scan(true);

    assert_eq!(
        report.duplicates, 0,
        "the other copy no longer exists, so there is nothing to decide about"
    );
    assert_eq!(
        harness.titles(),
        vec!["found again"],
        "the content is in the library exactly once"
    );

    // Same content, old path gone: this is the same recording somewhere else,
    // not a second one. Keeping the identity keeps its listening history and its
    // playlist entries.
    let moved = harness
        .media_files
        .find_by_path(&harness.music.join("found_again.wav"))
        .expect("looking it up")
        .expect("catalogued at the new path");
    assert_eq!(moved.state, FileState::Available);
    assert!(
        harness
            .media_files
            .find_by_path(&original)
            .expect("looking it up")
            .is_none(),
        "and nothing is left pointing at the old path"
    );
}

#[test]
fn subfolders_are_walked_only_when_asked() {
    let harness = harness("shallow");
    write_wav(&harness.music, "top.wav", 1, 10);
    write_wav(&harness.music.join("album"), "deep.wav", 1, 20);

    let report = harness.scan(false);

    assert_eq!(report.seen, 1, "only the top level was requested");
    assert_eq!(harness.titles(), vec!["top"]);
}

#[test]
fn files_that_are_not_music_are_ignored() {
    let harness = harness("ignore");
    write_wav(&harness.music, "song.wav", 1, 10);
    std::fs::write(harness.music.join("cover.jpg"), b"not audio").expect("a stray file");
    std::fs::write(harness.music.join("notes.txt"), b"not audio").expect("a stray file");

    let report = harness.scan(true);

    assert_eq!(report.seen, 1);
    assert_eq!(report.failed, 0, "a stray file is not a failure");
}

#[test]
fn tags_become_artists_albums_and_genres() {
    let harness = harness("tags");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");

    let report = harness.scan(true);
    assert_eq!(report.failed, 0);

    let tracks = harness.library.tracks().expect("listing");
    assert_eq!(tracks.len(), 1);
    assert_eq!(
        tracks[0].title, "Mysterons",
        "the tag wins over the filename"
    );
    assert!(
        tracks[0].artist_id.is_some(),
        "the artist tag should have produced an artist"
    );
    assert!(
        tracks[0].album_id.is_some(),
        "the album tag should have produced an album"
    );
}

/// Names of the genres the active profile sees for its only track.
fn genre_names(harness: &Harness) -> Vec<String> {
    let tracks = harness.library.tracks().expect("listing");
    let track = tracks.first().expect("one track");
    harness
        .library
        .genres_of(track.media_file_id)
        .expect("listing genres")
        .into_iter()
        .map(|genre| genre.name)
        .collect()
}

#[test]
fn a_listing_carries_names_rather_than_identifiers() {
    let harness = harness("summaries");
    let path = write_wav(&harness.music, "mysterons.wav", 2, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let listing = harness.library.summaries().expect("listing");
    assert_eq!(listing.len(), 1);

    let row = &listing[0];
    assert_eq!(row.title, "Mysterons");
    assert_eq!(row.artist.as_deref(), Some("Portishead"));
    assert_eq!(row.album.as_deref(), Some("Dummy"));
    assert_eq!(
        row.duration.as_millis(),
        2_000,
        "the length comes from the file, not from the library row"
    );
}

#[test]
fn a_listing_keeps_a_track_whose_tags_were_missing() {
    let harness = harness("summaries-untagged");
    write_wav(&harness.music, "01_unnamed.wav", 1, 10);
    harness.scan(true);

    let listing = harness.library.summaries().expect("listing");
    assert_eq!(
        listing.len(),
        1,
        "a left join, not an inner one: no artist must not mean no row"
    );
    assert!(listing[0].artist.is_none());
    assert!(listing[0].album.is_none());
}

#[test]
fn a_removed_track_leaves_the_listing() {
    let harness = harness("summaries-removed");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let media_file_id = harness.library.summaries().expect("listing")[0].media_file_id;
    harness
        .library
        .remove_track(media_file_id)
        .expect("removing");

    assert!(
        harness.library.summaries().expect("listing").is_empty(),
        "a tombstone is not part of the library"
    );
}

#[test]
fn correcting_a_genre_does_not_reach_the_other_profile() {
    let harness = harness("genre-leak");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    // Sasha disagrees with the tag.
    let tracks = harness.library.tracks().expect("listing");
    let media_file_id = tracks[0].media_file_id;
    harness
        .library
        .set_genres(media_file_id, &["Downtempo".to_owned()])
        .expect("correcting a genre");
    assert_eq!(genre_names(&harness), vec!["downtempo".to_owned()]);

    // Kim shares the machine, the folder and the file.
    let kim = harness.profiles.create("Kim").expect("a second profile");
    harness.profiles.switch_to(kim.id).expect("switching");
    harness.scan(true);

    assert_eq!(
        genre_names(&harness),
        vec!["trip-hop".to_owned()],
        "Kim sees what the file says, not what Sasha decided (PROJECT_MASTER 12.1)"
    );

    harness
        .profiles
        .switch_to(harness.profile_id)
        .expect("switching back");
    assert_eq!(
        genre_names(&harness),
        vec!["downtempo".to_owned()],
        "and Sasha still sees their own"
    );
}

#[test]
fn the_second_profile_to_import_a_file_gets_its_tags_too() {
    let harness = harness("second-profile-tags");
    let path = write_wav(&harness.music, "01_track.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let kim = harness.profiles.create("Kim").expect("a second profile");
    harness.profiles.switch_to(kim.id).expect("switching");
    harness.scan(true);

    let tracks = harness.library.tracks().expect("listing");
    assert_eq!(
        tracks[0].title, "Mysterons",
        "the file was already catalogued, but Kim is owed its tags, not its filename"
    );
    assert!(
        tracks[0].artist_id.is_some() && tracks[0].album_id.is_some(),
        "and its artist and album"
    );
}

#[test]
fn filing_a_track_under_nothing_is_a_decision_and_survives() {
    let harness = harness("genre-empty");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let media_file_id = harness.library.tracks().expect("listing")[0].media_file_id;
    harness
        .library
        .set_genres(media_file_id, &[])
        .expect("clearing every genre");

    assert!(
        genre_names(&harness).is_empty(),
        "an empty correction is not the same as having made none"
    );

    // A rescan re-reads the tags into the catalogue and must not undo it.
    harness.scan(true);
    assert!(genre_names(&harness).is_empty(), "and a rescan leaves it");
}

#[test]
fn resetting_a_genre_restores_what_the_file_says() {
    let harness = harness("genre-reset");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let media_file_id = harness.library.tracks().expect("listing")[0].media_file_id;
    harness
        .library
        .set_genres(media_file_id, &["Downtempo".to_owned()])
        .expect("correcting a genre");
    harness
        .library
        .reset_genres(media_file_id)
        .expect("dropping the correction");

    assert_eq!(genre_names(&harness), vec!["trip-hop".to_owned()]);
}

#[test]
fn a_genre_cannot_be_set_on_a_track_this_profile_does_not_have() {
    let harness = harness("genre-stranger");
    let path = write_wav(&harness.music, "mysterons.wav", 1, 10);
    tag_file(&path, "Mysterons", "Portishead", "Dummy", "Trip-Hop");
    harness.scan(true);

    let media_file_id = harness.library.tracks().expect("listing")[0].media_file_id;

    let kim = harness.profiles.create("Kim").expect("a second profile");
    harness.profiles.switch_to(kim.id).expect("switching");

    assert!(
        harness
            .library
            .set_genres(media_file_id, &["Downtempo".to_owned()])
            .is_err(),
        "the file is catalogued but not in Kim's library"
    );
}

#[test]
fn an_untagged_file_is_named_after_itself() {
    let harness = harness("untagged");
    write_wav(&harness.music, "01_Mysterons.wav", 1, 10);

    harness.scan(true);

    assert_eq!(
        harness.titles(),
        vec!["01 Mysterons"],
        "a hundred rows of Unknown would help nobody"
    );
}

#[test]
fn a_removed_track_is_not_resurrected_by_the_next_scan() {
    let harness = harness("removed");
    write_wav(&harness.music, "one.wav", 1, 10);
    harness.scan(true);

    let track = harness.library.tracks().expect("listing")[0].media_file_id;
    harness.library.remove_track(track).expect("removing");
    assert!(harness.titles().is_empty());

    harness.scan(true);
    assert!(
        harness.titles().is_empty(),
        "removing a track inside a scanned folder must mean something"
    );
}

#[test]
fn an_unreadable_file_does_not_stop_the_scan() {
    let harness = harness("broken");
    write_wav(&harness.music, "good.wav", 1, 10);
    // A supported extension over bytes that are not audio at all.
    std::fs::write(
        harness.music.join("broken.flac"),
        b"this is not a FLAC file",
    )
    .expect("a broken file");

    let report = harness.scan(true);

    assert_eq!(report.seen, 2);
    assert_eq!(report.added, 1, "the good file still imported");
    assert_eq!(report.failed, 1);
    assert_eq!(harness.titles(), vec!["good"]);
}

#[test]
fn a_file_deleted_while_cadenza_was_closed_is_noticed() {
    let harness = harness("refresh-missing");
    let path = write_wav(&harness.music, "gone.wav", 1, 55);
    harness.scan(true);

    // A scan only ever meets files that exist, so on its own it can never see a
    // deletion. This is the pass that closes that gap.
    std::fs::remove_file(&path).expect("deleting");
    assert_eq!(harness.library.refresh_missing().expect("refreshing"), 1);

    let file = harness
        .media_files
        .find_by_path(&path)
        .expect("looking it up")
        .expect("the row survives the file");
    assert_eq!(file.state, FileState::Missing);

    assert_eq!(
        harness.library.refresh_missing().expect("refreshing"),
        0,
        "a second pass over healthy rows must write nothing"
    );
}

#[test]
fn a_file_that_came_back_stops_being_missing() {
    let harness = harness("refresh-returned");
    let path = write_wav(&harness.music, "flaky.wav", 1, 56);
    harness.scan(true);

    let bytes = std::fs::read(&path).expect("reading");
    std::fs::remove_file(&path).expect("deleting");
    harness.library.refresh_missing().expect("refreshing");

    // A network drive reconnects, a removable disk comes back.
    std::fs::write(&path, bytes).expect("restoring");
    assert_eq!(harness.library.refresh_missing().expect("refreshing"), 1);

    let file = harness
        .media_files
        .find_by_path(&path)
        .expect("looking it up")
        .expect("catalogued");
    assert_eq!(file.state, FileState::Available);
}

#[test]
fn a_rename_moves_the_row_instead_of_replacing_it() {
    let harness = harness("rename");
    let from = write_wav(&harness.music, "old_name.wav", 1, 57);
    harness.scan(true);

    let before = harness
        .media_files
        .find_by_path(&from)
        .expect("looking it up")
        .expect("catalogued");

    let to = harness.music.join("new_name.wav");
    std::fs::rename(&from, &to).expect("renaming");
    harness
        .library
        .apply_change(&FileChange::Renamed {
            from: from.clone(),
            to: to.clone(),
        })
        .expect("applying the rename");

    let after = harness
        .media_files
        .find_by_path(&to)
        .expect("looking it up")
        .expect("catalogued at the new path");

    assert_eq!(
        after.id, before.id,
        "a moved file is the same recording; a new identity would take its \
         listening history and playlist entries with it"
    );
    assert_eq!(after.state, FileState::Available);
    assert!(
        harness
            .media_files
            .find_by_path(&from)
            .expect("looking it up")
            .is_none(),
        "and nothing is left behind at the old path"
    );
}

#[test]
fn a_rename_reported_as_a_removal_and_a_creation_still_moves_the_row() {
    // Found by running the watcher on Windows, which reports a rename as two
    // separate events. Treating the second as a new file left a phantom entry
    // pointing at nothing and a duplicate row beside it.
    let harness = harness("split-rename");
    let from = write_wav(&harness.music, "before.wav", 1, 60);
    harness.scan(true);
    let before = harness
        .media_files
        .find_by_path(&from)
        .expect("looking it up")
        .expect("catalogued");

    let to = harness.music.join("after.wav");
    std::fs::rename(&from, &to).expect("renaming");

    harness
        .library
        .apply_change(&FileChange::Removed(from.clone()))
        .expect("the removal half");
    harness
        .library
        .apply_change(&FileChange::Created(to.clone()))
        .expect("the creation half");

    let after = harness
        .media_files
        .find_by_path(&to)
        .expect("looking it up")
        .expect("catalogued at the new path");
    assert_eq!(after.id, before.id, "the same recording, moved");
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.state, FileState::Available);

    assert_eq!(
        harness.titles(),
        vec!["after"],
        "no phantom left behind at the old name"
    );
}

#[test]
fn a_removal_keeps_the_row_and_marks_it() {
    let harness = harness("watch-removal");
    let path = write_wav(&harness.music, "one.wav", 1, 58);
    harness.scan(true);

    std::fs::remove_file(&path).expect("deleting");
    harness
        .library
        .apply_change(&FileChange::Removed(path.clone()))
        .expect("applying the removal");

    let file = harness
        .media_files
        .find_by_path(&path)
        .expect("looking it up")
        .expect("the row survives, carrying history and playlist entries");
    assert_eq!(file.state, FileState::Missing);
}

#[test]
fn a_file_dropped_into_a_watched_folder_is_imported() {
    let harness = harness("watch-create");
    harness.scan(true);
    assert!(harness.titles().is_empty());

    let path = write_wav(&harness.music, "dropped.wav", 1, 59);
    harness
        .library
        .apply_change(&FileChange::Created(path))
        .expect("applying the creation");

    assert_eq!(harness.titles(), vec!["dropped"]);
}

#[test]
fn changes_to_things_that_are_not_music_are_shrugged_off() {
    let harness = harness("watch-noise");
    harness.scan(true);

    // The watcher reports everything under the folder, including cover art the
    // listener drops in and files that vanish before anything reads them.
    let cover = harness.music.join("cover.jpg");
    std::fs::write(&cover, b"not audio").expect("a stray file");

    harness
        .library
        .apply_change(&FileChange::Created(cover))
        .expect("a stray file is nothing to do");
    harness
        .library
        .apply_change(&FileChange::Removed(
            harness.music.join("never-existed.wav"),
        ))
        .expect("a removal of something unknown is nothing to do");
    harness
        .library
        .apply_change(&FileChange::Modified(harness.music.join("gone.wav")))
        .expect("a file that vanished before we looked is nothing to do");

    assert!(harness.titles().is_empty());
}

#[test]
fn adding_something_that_is_not_a_folder_is_refused() {
    let harness = harness("not-a-folder");
    let file = write_wav(&harness.music, "one.wav", 1, 10);

    assert!(
        harness.library.add_folder(&file, true).is_err(),
        "a typo must be reported now, not as an empty library later"
    );
    assert!(
        harness
            .library
            .add_folder(&harness.music.join("nowhere"), true)
            .is_err()
    );
}

#[test]
fn the_folder_records_when_it_was_last_scanned() {
    let harness = harness("last-scan");
    write_wav(&harness.music, "one.wav", 1, 10);

    assert!(
        harness
            .library
            .add_folder(&harness.music, true)
            .expect("adding")
            .last_scan_at
            .is_none()
    );

    harness.scan(true);

    let folder = harness.library.folders().expect("listing")[0].clone();
    assert!(
        folder.last_scan_at.is_some(),
        "a scan that leaves no trace cannot be resumed or reported"
    );
    assert_eq!(folder.profile_id, harness.profile_id);
}

#[test]
fn a_listener_can_correct_a_track_without_touching_the_file_or_anyone_else() {
    let harness = harness("edit");
    write_wav(&harness.music, "one.wav", 1, 10);
    harness.scan(true);

    let track = harness
        .library
        .summaries()
        .expect("a library")
        .into_iter()
        .next()
        .expect("something was imported");

    harness
        .library
        .edit_track(
            track.media_file_id,
            "Mysterons",
            Some("Portishead"),
            Some("Dummy"),
        )
        .expect("corrected");

    let corrected = harness
        .library
        .summaries()
        .expect("a library")
        .into_iter()
        .find(|row| row.media_file_id == track.media_file_id)
        .expect("still there");

    assert_eq!(corrected.title, "Mysterons");
    assert_eq!(corrected.artist.as_deref(), Some("Portishead"));
    assert_eq!(corrected.album.as_deref(), Some("Dummy"));

    // The file itself was not touched: a rescan finds nothing to update, and
    // what the listener called it survives (PROJECT_MASTER 2.1).
    let report = harness.scan(true);
    assert_eq!(report.added, 0);
    assert_eq!(
        harness
            .library
            .summaries()
            .expect("a library")
            .into_iter()
            .find(|row| row.media_file_id == track.media_file_id)
            .expect("still there")
            .title,
        "Mysterons",
        "a scan must not undo a correction"
    );

    // Clearing a field means the absence, not a name made of spaces.
    harness
        .library
        .edit_track(track.media_file_id, "Mysterons", Some("   "), None)
        .expect("cleared");

    let cleared = harness
        .library
        .summaries()
        .expect("a library")
        .into_iter()
        .find(|row| row.media_file_id == track.media_file_id)
        .expect("still there");
    assert_eq!(cleared.artist, None);
    assert_eq!(cleared.album, None);

    // A track has to be called something.
    assert!(
        harness
            .library
            .edit_track(track.media_file_id, "  ", None, None)
            .is_err()
    );
}
