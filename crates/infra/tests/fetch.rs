//! Bringing a track in from a link: the decisions, not the download.
//!
//! Nothing here reaches a network, and that is the point of the fake. What the
//! real adapter does — starting `yt-dlp`, reading its progress, moving what it
//! left behind — is tested by its own unit tests and, in the end, by using it.
//! What is worth pinning down is the order the service refuses things in, and
//! that a refusal happens *before* anything runs: a link that was never a link
//! must not start a program, and a machine with nowhere to put a track must be
//! told so rather than after ten seconds of downloading.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cadenza_core::application::services::{Fetched, LibraryPorts, LibraryService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ports::fetcher::{FetchPort, MissingTool};
use cadenza_core::domain::ports::folder_picker::FolderPickerPort;
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

/// A downloader that never leaves the machine.
///
/// It writes a real file, because everything after the fetch is the ordinary
/// import and that has to run for the test to mean anything. A wav rather than
/// an mp3: what comes back is whatever the tool produced, and the service reads
/// it with the same reader it reads every other file with.
#[derive(Default)]
struct FakeFetcher {
    /// What to report as absent, so the "install these" path can be reached.
    missing: Vec<MissingTool>,
    /// Every link it was actually asked to fetch.
    asked: Mutex<Vec<String>>,
    /// Set once it has run, so a test can prove it did not.
    ran: AtomicBool,
}

impl FetchPort for FakeFetcher {
    fn missing(&self) -> Vec<MissingTool> {
        self.missing.clone()
    }

    fn fetch(
        &self,
        link: &str,
        into: &Path,
        progress: &dyn Fn(u8),
    ) -> cadenza_core::Result<PathBuf> {
        self.ran.store(true, Ordering::Relaxed);
        self.asked.lock().expect("the record").push(link.to_owned());

        progress(50);
        let landed = write_wav(into, "A Fetched Track.wav", 1, 900);
        progress(100);

        Ok(landed)
    }
}

/// A machine whose music folder is somewhere this test controls.
struct Suggesting(PathBuf);

impl FolderPickerPort for Suggesting {
    fn pick_folder(&self, _title: &str) -> cadenza_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn pick_image(&self, _title: &str) -> cadenza_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn suggested_music_folder(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

struct Harness {
    library: LibraryService,
    fetcher: Arc<FakeFetcher>,
    /// Where the suggestion points, which is where a fetched track must land.
    local: PathBuf,
    /// Last, so nothing still holds the database when the directory goes.
    _db: TempDb,
}

fn harness(tag: &str, fetcher: FakeFetcher) -> Harness {
    let db = TempDb::new();
    let root = db.directory().join(tag);
    let local = root.join("Music").join("Cadenza");

    let context = Arc::new(AppContext::new(
        Arc::new(TestClock::default()),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(db.pool().clone())),
        Arc::new(SqliteSettingsRepository::new(db.pool().clone())),
    ));
    ProfileService::new(Arc::clone(&context))
        .create("Sasha")
        .expect("a profile");

    let fetcher = Arc::new(fetcher);
    let ports = LibraryPorts {
        picker: Arc::new(Suggesting(local.clone())),
        files: Arc::new(LocalFileSystem),
        metadata: Arc::new(LoftyMetadataReader),
        artwork: Arc::new(FileArtworkCache::new(root.join("artwork")).expect("a cache")),
        media_files: Arc::new(SqliteMediaFileRepository::new(db.pool().clone())),
        tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
        artists: Arc::new(SqliteArtistRepository::new(db.pool().clone())),
        albums: Arc::new(SqliteAlbumRepository::new(db.pool().clone())),
        genres: Arc::new(SqliteGenreRepository::new(db.pool().clone())),
        reviews: Arc::new(SqliteImportReviewRepository::new(db.pool().clone())),
        watcher: None,
        fetcher: Some(Arc::clone(&fetcher) as _),
    };

    Harness {
        library: LibraryService::new(context, ports),
        fetcher,
        local,
        _db: db,
    }
}

/// Nothing at all, for the tests that only care that it was never reached.
fn nothing(_percent: u8) {}

#[test]
fn a_track_from_a_link_lands_in_the_local_folder_and_joins_the_library() {
    let harness = harness("landing", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    // A cell, because the port hands progress to an `Fn`: it may be called
    // from anywhere and any number of times, which is exactly what a download
    // does and exactly what a `FnMut` could not promise.
    let seen = std::cell::RefCell::new(Vec::new());
    let outcome = harness
        .library
        .fetch_from_link("https://example.com/watch?v=abc", &|percent| {
            seen.borrow_mut().push(percent);
        })
        .expect("a fetch");

    assert_eq!(
        outcome,
        Fetched::Landed("A Fetched Track".to_owned()),
        "it says what arrived"
    );
    assert!(
        harness.local.join("A Fetched Track.wav").is_file(),
        "and it is in Cadenza's own folder, not somewhere else"
    );

    let tracks = harness.library.tracks().expect("the library");
    assert_eq!(tracks.len(), 1, "a fetched track is a track like any other");

    assert_eq!(
        seen.into_inner(),
        vec![50, 100],
        "progress reached whoever asked for it"
    );
    assert_eq!(
        harness.fetcher.asked.lock().expect("the record").as_slice(),
        ["https://example.com/watch?v=abc"],
        "the link went through as it was given"
    );
}

#[test]
fn without_a_local_folder_the_offer_to_make_one_comes_back() {
    // The listener said no when Cadenza offered to make itself a folder. That
    // was a fair answer then; now there is a reason, and the answer is the
    // offer again rather than a failure.
    let harness = harness("nowhere", FakeFetcher::default());

    let outcome = harness
        .library
        .fetch_from_link("https://example.com/a", &nothing)
        .expect("an outcome rather than an error");

    assert_eq!(
        outcome,
        Fetched::NeedsLocalFolder(harness.local.clone()),
        "and it names the folder it would make"
    );
    assert!(
        !harness.fetcher.ran.load(Ordering::Relaxed),
        "nothing was downloaded to be thrown away"
    );
}

#[test]
fn a_machine_without_the_programs_is_told_which_ones() {
    let harness = harness(
        "toolless",
        FakeFetcher {
            missing: vec![MissingTool {
                name: "yt-dlp".to_owned(),
                reason: "fetches what is behind the link".to_owned(),
            }],
            ..FakeFetcher::default()
        },
    );
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    let outcome = harness
        .library
        .fetch_from_link("https://example.com/a", &nothing)
        .expect("an outcome rather than an error");

    let Fetched::NeedsTools(missing) = outcome else {
        panic!("the missing programs should be named: {outcome:?}");
    };
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].name, "yt-dlp");
    assert!(!harness.fetcher.ran.load(Ordering::Relaxed));
}

#[test]
fn what_is_not_a_link_never_reaches_the_downloader() {
    // The reason the check exists: what is typed here becomes an argument to
    // another program, and an argument that can turn into a flag is a text
    // field that runs things.
    let harness = harness("refusal", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    for typed in [
        "",
        "not a link",
        "--exec calc.exe",
        "C:\\Windows\\System32\\calc.exe",
        "https://example.com/a --exec calc.exe",
    ] {
        assert!(
            harness.library.fetch_from_link(typed, &nothing).is_err(),
            "{typed:?} should have been refused"
        );
    }

    assert!(
        !harness.fetcher.ran.load(Ordering::Relaxed),
        "and none of them started anything"
    );
}
