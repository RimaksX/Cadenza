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

use cadenza_core::application::services::{
    Fetched, LibraryPorts, LibraryService, PlaylistPorts, PlaylistService,
};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::ports::fetcher::{
    FetchPort, FetchProgress, FetchWhat, FetchedTracks, MissingTool,
};
use cadenza_core::domain::ports::folder_picker::FolderPickerPort;
use cadenza_infra::db::repositories::{
    SqliteAlbumRepository, SqliteArtistRepository, SqliteGenreRepository,
    SqliteImportReviewRepository, SqliteMediaFileRepository, SqlitePlaylistRepository,
    SqliteProfileRepository, SqliteSettingsRepository, SqliteTrackRepository,
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
    /// Set once the missing programs have been installed through it.
    installed: Mutex<bool>,
    /// What it refuses with, where a test is about a refusal.
    refuse: Option<String>,
}

impl FetchPort for FakeFetcher {
    fn missing(&self) -> Vec<MissingTool> {
        self.missing.clone()
    }

    fn install(&self, said: &dyn Fn(&str)) -> cadenza_core::Result<Vec<MissingTool>> {
        said("installing");
        // A fake package manager that always works, so that what is under test
        // is what the service does about it rather than what winget does.
        *self.installed.lock().expect("the record") = true;
        Ok(Vec::new())
    }

    fn update(&self, said: &dyn Fn(&str)) -> cadenza_core::Result<String> {
        said("updating");
        Ok("yt-dlp is up to date".to_owned())
    }

    fn fetch(
        &self,
        link: &str,
        into: &Path,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> cadenza_core::Result<FetchedTracks> {
        self.ran.store(true, Ordering::Relaxed);
        self.asked.lock().expect("the record").push(link.to_owned());

        if let Some(refusal) = self.refuse.as_deref() {
            return Err(cadenza_core::CoreError::invalid("link", refusal));
        }

        // A listener who pressed stop before anything started gets what a
        // listener who pressed stop before anything started should get.
        if stop() {
            return Ok(FetchedTracks::default());
        }

        let how_many = match what {
            FetchWhat::OneTrack => 1,
            FetchWhat::WholePlaylist => 3,
        };

        let mut landed = Vec::new();
        for index in 1..=how_many {
            progress(FetchProgress {
                percent: 50,
                item: (how_many > 1).then_some((index, how_many)),
            });
            // Numbered only where there are several, so that the one-track
            // case is still called what every other test calls it.
            let name = if how_many == 1 {
                "A Fetched Track.wav".to_owned()
            } else {
                format!("A Fetched Track {index}.wav")
            };
            // A different fill per track, so that three of them are three
            // recordings rather than one file written three times: the import
            // hashes what it is given, and identical files are a duplicate by
            // every measure it has.
            landed.push(write_wav(
                into,
                &name,
                1,
                900 + i16::try_from(index).expect("a small playlist"),
            ));
        }
        progress(FetchProgress {
            percent: 100,
            item: None,
        });

        Ok(FetchedTracks {
            files: landed,
            // Named only when a playlist is what was asked for, the way the
            // downloader only prints a name when there is one.
            playlist: matches!(what, FetchWhat::WholePlaylist)
                .then(|| "A Fetched Playlist".to_owned()),
        })
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
    /// The same service the library was handed, so a test can ask what it made.
    playlists: Arc<PlaylistService>,
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
    let playlists = Arc::new(PlaylistService::new(
        Arc::clone(&context),
        PlaylistPorts {
            playlists: Arc::new(SqlitePlaylistRepository::new(db.pool().clone())),
            tracks: Arc::new(SqliteTrackRepository::new(db.pool().clone())),
            artwork: Arc::new(FileArtworkCache::new(db.directory().join("art")).expect("a cache")),
            picker: Arc::new(Suggesting(local.clone())),
            files: Arc::new(LocalFileSystem),
        },
    ));

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
        playlists: Some(Arc::clone(&playlists)),
    };

    Harness {
        library: LibraryService::new(context, ports),
        playlists,
        fetcher,
        local,
        _db: db,
    }
}

/// Nothing at all, for the tests that only care that it was never reached.
fn nothing(_report: FetchProgress) {}

/// A listener who is not pressing stop, which is almost all of them.
fn carry_on() -> bool {
    false
}

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
        .fetch_from_link(
            "https://example.com/watch?v=abc",
            FetchWhat::OneTrack,
            &|report| seen.borrow_mut().push(report.percent),
            &carry_on,
        )
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
        .fetch_from_link(
            "https://example.com/a",
            FetchWhat::OneTrack,
            &nothing,
            &carry_on,
        )
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
                install: "winget install yt-dlp.yt-dlp".to_owned(),
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
        .fetch_from_link(
            "https://example.com/a",
            FetchWhat::OneTrack,
            &nothing,
            &carry_on,
        )
        .expect("an outcome rather than an error");

    let Fetched::NeedsTools(missing) = outcome else {
        panic!("the missing programs should be named: {outcome:?}");
    };
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].name, "yt-dlp");
    assert!(!harness.fetcher.ran.load(Ordering::Relaxed));
}

#[test]
fn a_service_that_encrypts_its_audio_is_refused_by_name() {
    // Not a failure to hide behind a generic message: no version of any tool
    // will ever fetch these, and saying which service it is turns ten seconds
    // of waiting into one sentence somebody can act on.
    let harness = harness("locked", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    let refused = harness
        .library
        .fetch_from_link(
            "https://open.spotify.com/track/abc",
            FetchWhat::OneTrack,
            &nothing,
            &carry_on,
        )
        .expect_err("Spotify cannot be fetched from");

    assert!(
        refused.to_string().contains("Spotify"),
        "the service is named: {refused}"
    );
    assert!(
        !harness.fetcher.ran.load(Ordering::Relaxed),
        "and nothing was started to find that out"
    );
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
            harness
                .library
                .fetch_from_link(typed, FetchWhat::OneTrack, &nothing, &carry_on)
                .is_err(),
            "{typed:?} should have been refused"
        );
    }

    assert!(
        !harness.fetcher.ran.load(Ordering::Relaxed),
        "and none of them started anything"
    );
}

#[test]
fn a_playlist_link_brings_in_everything_behind_it() {
    let harness = harness("playlist", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    let outcome = harness
        .library
        .fetch_from_link(
            "https://example.com/watch?v=abc&list=xyz",
            FetchWhat::WholePlaylist,
            &nothing,
            &carry_on,
        )
        .expect("a fetch");

    // Counted rather than named: forty names is not a thing a line under a
    // button can say, and the library below is already showing them.
    assert_eq!(outcome, Fetched::LandedMany(3));
    assert_eq!(
        harness.library.tracks().expect("the library").len(),
        3,
        "and all of them joined the library, not just the first"
    );
}

#[test]
fn stopping_leaves_the_library_where_it_was() {
    let harness = harness("stopped", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    let outcome = harness
        .library
        .fetch_from_link(
            "https://example.com/watch?v=abc&list=xyz",
            FetchWhat::WholePlaylist,
            &nothing,
            // Already pressed by the time the downloader starts, which is the
            // hardest moment for it to be pressed.
            &|| true,
        )
        .expect("stopping is not a failure");

    assert_eq!(outcome, Fetched::NothingNew);
    assert!(
        harness.library.tracks().expect("the library").is_empty(),
        "nothing half-fetched was imported"
    );
}

#[test]
fn a_playlist_that_arrived_as_one_becomes_one() {
    let harness = harness("gathered", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    harness
        .library
        .fetch_from_link(
            "https://example.com/watch?v=abc&list=xyz",
            FetchWhat::WholePlaylist,
            &nothing,
            &carry_on,
        )
        .expect("a fetch");

    // Forty tracks landing loose in a library is forty tracks somebody has to
    // gather up by hand, and the thing they were part of is what they pasted.
    let made = harness.playlists.list().expect("the playlists");
    let [only] = made.as_slice() else {
        panic!("one playlist arrived and {} were made", made.len());
    };
    assert_eq!(only.playlist.name.as_str(), "A Fetched Playlist");
    assert_eq!(only.track_count, 3, "with everything that came with it");
}

#[test]
fn one_track_makes_no_playlist() {
    let harness = harness("ungathered", FakeFetcher::default());
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    harness
        .library
        .fetch_from_link(
            "https://example.com/watch?v=abc",
            FetchWhat::OneTrack,
            &nothing,
            &carry_on,
        )
        .expect("a fetch");

    assert!(
        harness.playlists.list().expect("the playlists").is_empty(),
        "one track is a track, not a list of one"
    );
}

#[test]
fn a_refusal_that_reads_like_a_stale_copy_becomes_an_offer() {
    let harness = harness(
        "stale",
        FakeFetcher {
            refuse: Some("unable to download video data: HTTP Error 403: Forbidden".to_owned()),
            ..FakeFetcher::default()
        },
    );
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    let outcome = harness
        .library
        .fetch_from_link(
            "https://example.com/watch?v=abc",
            FetchWhat::OneTrack,
            &nothing,
            &carry_on,
        )
        .expect("a refusal that can be answered is not an error");

    // What it said is still there. The offer is what is added to it, not what
    // replaces it: "403" is the truest thing anybody can be told about this.
    let Fetched::NeedsUpdate(said) = outcome else {
        panic!("a stale-looking refusal should offer an update, and gave {outcome:?}");
    };
    assert!(said.contains("403"), "and it still says what happened");
}

#[test]
fn a_refusal_about_the_link_is_not_an_offer_to_update() {
    let harness = harness(
        "private",
        FakeFetcher {
            refuse: Some("Video unavailable. This video is private".to_owned()),
            ..FakeFetcher::default()
        },
    );
    harness
        .library
        .use_suggested_folder()
        .expect("the local folder");

    // No amount of updating answers this one, and an offer that never works is
    // an offer nobody reads by the third time.
    assert!(
        harness
            .library
            .fetch_from_link(
                "https://example.com/watch?v=abc",
                FetchWhat::OneTrack,
                &nothing,
                &carry_on,
            )
            .is_err()
    );
}

#[test]
fn installing_clears_what_was_missing() {
    let harness = harness(
        "toolless",
        FakeFetcher {
            missing: vec![MissingTool {
                name: "yt-dlp".to_owned(),
                install: "winget install yt-dlp.yt-dlp".to_owned(),
            }],
            ..FakeFetcher::default()
        },
    );

    let told = std::cell::RefCell::new(Vec::new());
    let still_missing = harness
        .library
        .install_tools(&|line| told.borrow_mut().push(line.to_owned()))
        .expect("an install");

    assert!(still_missing.is_empty(), "nothing is missing afterwards");
    assert!(
        !told.borrow().is_empty(),
        "and it said what it was doing while it did it"
    );
}
