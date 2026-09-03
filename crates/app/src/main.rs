//! Cadenza composition root.
//!
//! The only place that constructs concrete infrastructure and hands it to the
//! application layer as port implementations (PROJECT_MASTER 4.4).
//!
//! Wiring lives here rather than in a `wiring.rs` of its own: there are four
//! dependencies. It gets its own file when it earns one.
//!
//! There is one thing to run and no arguments to say so. The command line this
//! file used to carry was scaffolding for the milestones before the window
//! existed — a way to create a profile, add a folder or hear a file when there
//! was nothing to click. Every one of those is now something the interface
//! does, and a released player has no business asking for a terminal.

// No console. A double-clicked player that opens a black window beside itself
// is telling the listener about its own implementation, and there is nothing
// left in this binary that writes to one anyway. What used to go to a terminal
// now goes to the log, or — for the two failures that stop the application
// existing at all — to a dialog, because a program that cannot start must say
// so rather than fail to appear.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use cadenza_core::application::services::{
    AnalysisPorts, AnalysisService, EqPorts, EqService, LibraryPorts, LibraryService,
    PlaybackPorts, PlaybackService, PlaylistPorts, PlaylistService, QueuePorts, QueueService,
    RadioPorts, RadioService, StatsPorts, StatsService,
};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::playback::PlaybackState;
use cadenza_core::domain::ports::artwork_cache::ArtworkCachePort;
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::event_bus::EventBusPort;
use cadenza_core::domain::ports::file_watcher::FileWatcherPort;
use cadenza_core::domain::ports::log::{LogLevel, LogPort};
use cadenza_infra::analysis::{AnalysisWorker, DspFeatureExtractor};
use cadenza_infra::audio::CpalAudioEngine;
use cadenza_infra::db;
use cadenza_infra::db::repositories::{
    SqliteAlbumRepository, SqliteAnalysisJobRepository, SqliteArtistRepository,
    SqliteEqPresetRepository, SqliteGenreRepository, SqliteHistoryRepository,
    SqliteImportReviewRepository, SqliteMediaFileRepository, SqliteMoodRepository,
    SqlitePlaylistRepository, SqliteProfileRepository, SqliteQueueRepository,
    SqliteRadioRepository, SqliteSettingsRepository, SqliteTrackFeaturesRepository,
    SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_infra::library::{LocalFileSystem, NotifyFileWatcher};
use cadenza_infra::metadata::{FileArtworkCache, LoftyMetadataReader};
use cadenza_infra::system::{
    AppPaths, FileLog, SystemClock, SystemFolderPicker, WindowsPriority, alert,
};

fn main() -> ExitCode {
    // A crash used to print to a terminal that was open beside the window.
    // There is no terminal now, so a panic would end the process with nothing
    // on screen at all — the listener's music simply stopping, with no way to
    // tell it from a machine that fell asleep.
    std::panic::set_hook(Box::new(|panic| {
        alert::fatal(&format!(
            "Cadenza has stopped and cannot carry on.\n\n{panic}"
        ));
    }));

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            alert::fatal(&format!("Cadenza could not start.\n\n{message}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> std::result::Result<(), String> {
    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;
    paths.ensure_directories().map_err(|err| err.to_string())?;

    // Opening the database also migrates it, so the schema is current before any
    // adapter is handed a connection.
    let pool = db::open(&paths.database()).map_err(|err| err.to_string())?;

    // Held rather than passed in place: the window subscribes to the same bus
    // the services publish on, which is how it hears about a change it did not
    // make itself.
    let events: Arc<dyn EventBusPort> = Arc::new(InProcessEventBus::new());

    let watcher: Arc<dyn FileWatcherPort> =
        Arc::new(NotifyFileWatcher::new().map_err(|err| err.to_string())?);

    let log: Arc<dyn LogPort> = Arc::new(
        FileLog::new(
            paths.log_file(),
            paths.previous_log_file(),
            Arc::new(SystemClock),
        )
        .map_err(|err| err.to_string())?,
    );

    let context = Arc::new(
        AppContext::new(
            Arc::new(SystemClock),
            Arc::clone(&events),
            Arc::new(SqliteProfileRepository::new(pool.clone())),
            Arc::new(SqliteSettingsRepository::new(pool.clone())),
        )
        .with_log(Arc::clone(&log)),
    );
    let profiles = Arc::new(ProfileService::new(Arc::clone(&context)));

    // One cache, shared: a cover belongs to a recording or to a list, and both
    // of those live in the same directory under %LOCALAPPDATA%.
    let artwork: Arc<dyn ArtworkCachePort> =
        Arc::new(FileArtworkCache::new(paths.artwork_cache_dir()).map_err(|err| err.to_string())?);

    let library = Arc::new(LibraryService::new(
        Arc::clone(&context),
        LibraryPorts {
            files: Arc::new(LocalFileSystem),
            metadata: Arc::new(LoftyMetadataReader),
            artwork: Arc::clone(&artwork),
            media_files: Arc::new(SqliteMediaFileRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            artists: Arc::new(SqliteArtistRepository::new(pool.clone())),
            albums: Arc::new(SqliteAlbumRepository::new(pool.clone())),
            genres: Arc::new(SqliteGenreRepository::new(pool.clone())),
            reviews: Arc::new(SqliteImportReviewRepository::new(pool.clone())),
            picker: Arc::new(SystemFolderPicker),
            watcher: Some(Arc::clone(&watcher)),
        },
    ));

    let playlists = Arc::new(PlaylistService::new(
        Arc::clone(&context),
        PlaylistPorts {
            playlists: Arc::new(SqlitePlaylistRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            artwork: Arc::clone(&artwork),
            picker: Arc::new(SystemFolderPicker),
            files: Arc::new(LocalFileSystem),
        },
    ));

    // Analysis needs no profile and no audio device: what a recording sounds
    // like is a fact about the file, and every listener shares it.
    let analysis = Arc::new(AnalysisService::new(
        Arc::clone(&context),
        AnalysisPorts {
            jobs: Arc::new(SqliteAnalysisJobRepository::new(pool.clone())),
            features: Arc::new(SqliteTrackFeaturesRepository::new(pool.clone())),
            media_files: Arc::new(SqliteMediaFileRepository::new(pool.clone())),
            extractor: Arc::new(DspFeatureExtractor::new(Arc::new(SystemClock))),
        },
    ));

    // Radio needs no audio device either: a station is a list of picks, and
    // playing them is the queue's business.
    let radio = Arc::new(RadioService::new(
        Arc::clone(&context),
        RadioPorts {
            radio: Arc::new(SqliteRadioRepository::new(pool.clone())),
            moods: Arc::new(SqliteMoodRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            features: Arc::new(SqliteTrackFeaturesRepository::new(pool.clone())),
            stats: Arc::new(SqliteHistoryRepository::new(pool.clone())),
        },
    ));

    // Statistics and the retention that bounds them.
    let stats = Arc::new(StatsService::new(
        Arc::clone(&context),
        StatsPorts {
            history: Arc::new(SqliteHistoryRepository::new(pool.clone())),
            stats: Arc::new(SqliteHistoryRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
        },
    ));

    // Thirty days is a promise about what is on the disk, so it is kept on the
    // way in rather than when somebody opens a screen. A failure here is not a
    // reason to refuse to start: the listener came to play music.
    match stats.purge_expired() {
        Ok(removed) if removed > 0 => log.write(
            LogLevel::Info,
            &format!("cleared {removed} listening event(s) older than thirty days"),
        ),
        Ok(_) => {}
        Err(err) => log.write(
            LogLevel::Warn,
            &format!("the thirty-day sweep did not run: {err}"),
        ),
    }

    // Before anything else: the pointer left by the previous run decides who the
    // application is running as.
    let active = profiles.restore_active().map_err(|err| err.to_string())?;

    // The audio device is opened here and nowhere else.
    // One engine, shared: the equaliser and the transport are two things asked
    // of the same filters.
    let engine: Arc<dyn AudioEnginePort> =
        Arc::new(CpalAudioEngine::new().map_err(|err| err.to_string())?);

    let playback = Arc::new(PlaybackService::new(
        Arc::clone(&context),
        PlaybackPorts {
            engine: Arc::clone(&engine),
            media_files: Arc::new(SqliteMediaFileRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            history: Arc::new(SqliteHistoryRepository::new(pool.clone())),
        },
    ));

    // Built after the profile has been restored, because building it is what
    // restores that profile's queue.
    let queue = Arc::new(QueueService::new(
        Arc::clone(&context),
        Arc::clone(&playback),
        QueuePorts {
            queue: Arc::new(SqliteQueueRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            features: Arc::new(SqliteTrackFeaturesRepository::new(pool.clone())),
            radio: Some(Arc::clone(&radio)),
        },
    ));

    let eq = Arc::new(EqService::new(
        Arc::clone(&context),
        EqPorts {
            presets: Arc::new(SqliteEqPresetRepository::new(pool.clone())),
            engine: Arc::clone(&engine),
        },
    ));

    // Analysis runs for as long as the window is open and stops with it.
    // Nothing waits for it: a listener who opens Cadenza to hear something
    // hears it now, and the library learns what it sounds like behind them
    // (PROJECT_MASTER 2.11).
    let mut analyser = AnalysisWorker::start(
        Arc::clone(&analysis),
        Arc::new(WindowsPriority),
        Arc::new({
            let engine = Arc::clone(&engine);
            move || engine.state() == PlaybackState::Playing
        }),
        Arc::clone(&log),
    );

    context.info(&format!(
        "Cadenza {} started as {}",
        env!("CARGO_PKG_VERSION"),
        active
            .as_ref()
            .map_or_else(|| "nobody".to_owned(), |profile| profile.name.to_string())
    ));

    // The library keeps itself current for as long as the window is open, which
    // is what "автоматическое отслеживание изменений файловой системы" asks for.
    watcher.set_handler(Box::new({
        let library = Arc::clone(&library);
        let log = Arc::clone(&log);
        move |change| {
            // A failure concerns one file, and tearing the watcher down over an
            // unreadable download would cost the listener every other file. The
            // window is on another thread and cannot be told, so the log is
            // told instead.
            if let Err(err) = library.apply_change(&change) {
                log.write(
                    LogLevel::Warn,
                    &format!("a change on disk was not applied: {err}"),
                );
            }
        }
    }));
    library.watch_folders().map_err(|err| err.to_string())?;

    // And a look at the folders themselves, because the watcher only knows what
    // happened while it was watching: music copied in while Cadenza was closed
    // is otherwise found by nobody until somebody presses a button they have no
    // reason to press (MASTER_ISSUES 68).
    //
    // On a thread, because the window should open now and this is a stat and an
    // indexed lookup per file. It publishes what it finds, and the window hears
    // it the same way it hears the watcher.
    std::thread::spawn({
        let library = Arc::clone(&library);
        let log = Arc::clone(&log);
        move || match library.scan_all() {
            Ok(report) if report.added + report.updated > 0 => log.write(
                LogLevel::Info,
                &format!(
                    "the opening scan found {} new and {} changed",
                    report.added, report.updated
                ),
            ),
            Ok(_) => {}
            Err(err) => log.write(LogLevel::Warn, &format!("the opening scan stopped: {err}")),
        }
    });

    let outcome = cadenza_ui::run(cadenza_ui::UiServices {
        profiles: Arc::clone(&profiles),
        library: Arc::clone(&library),
        eq,
        playback,
        queue,
        playlists,
        radio: Arc::clone(&radio),
        stats: Arc::clone(&stats),
        profile: active,
        events: Arc::clone(&events),
    });

    // Before the pool goes: the worker holds a connection, and a thread still
    // decoding while the process exits is a file left open.
    analyser.stop();
    outcome.map_err(|err| err.to_string())
}
