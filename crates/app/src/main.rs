//! Cadenza composition root.
//!
//! The only place that constructs concrete infrastructure and hands it to the
//! application layer as port implementations (PROJECT_MASTER 4.4).
//!
//! Wiring lives here rather than in a `wiring.rs` of its own: there are four
//! dependencies. It gets its own file when it earns one.

#![forbid(unsafe_code)]

mod cli;

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::{env, io};

use cadenza_core::application::services::{
    AnalysisPorts, AnalysisService, EqPorts, EqService, LibraryPorts, LibraryService,
    PlaybackPorts, PlaybackService, PlaylistPorts, PlaylistService, QueuePorts, QueueService,
    RadioPorts, RadioService, StatsPorts, StatsService,
};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::playback::PlaybackState;
use cadenza_core::domain::ports::artwork_cache::ArtworkCachePort;
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::decoder::DecoderPort;
use cadenza_core::domain::ports::event_bus::EventBusPort;
use cadenza_core::domain::ports::file_watcher::FileWatcherPort;
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::settings::{
    CROSSFADE_ENABLED_KEY, CROSSFADE_MS_KEY, CrossfadeDuration, SettingValue,
};
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;
use cadenza_core::domain::value_objects::{DurationMs, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use cadenza_infra::analysis::{AnalysisWorker, DspFeatureExtractor};
use cadenza_infra::audio::{CpalAudioEngine, SymphoniaDecoder};
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
use cadenza_infra::system::{AppPaths, SystemClock, SystemFolderPicker, WindowsPriority};

use cli::{Command, PlaylistCommand};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cadenza: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> std::result::Result<(), String> {
    let command =
        cli::parse(env::args().skip(1)).map_err(|err| format!("{err}\n\n{}", cli::USAGE))?;

    if command == Command::Help {
        println!("{}", cli::USAGE);
        return Ok(());
    }

    // Playing a file by path needs neither a profile nor the database: it is the
    // audio engine on its own, which is exactly what M5 is for.
    if let Command::Play(path) = &command {
        return play(Path::new(path)).map_err(|err| err.to_string());
    }

    let paths = AppPaths::resolve().map_err(|err| err.to_string())?;

    if command == Command::Paths {
        report_paths(&paths);
        return Ok(());
    }

    paths.ensure_directories().map_err(|err| err.to_string())?;

    // Opening the database also migrates it, so the schema is current before any
    // adapter is handed a connection.
    let pool = db::open(&paths.database()).map_err(|err| err.to_string())?;

    // Held rather than passed in place: the window subscribes to the same bus
    // the services publish on, which is how it hears about a change it did not
    // make itself.
    let events: Arc<dyn EventBusPort> = Arc::new(InProcessEventBus::new());

    // Only the window watches. A command that scans once and exits would start a
    // thread, register directories with the operating system and tear both down
    // again before anything could happen in them.
    let watcher: Option<Arc<dyn FileWatcherPort>> = if command == Command::Ui {
        Some(Arc::new(
            NotifyFileWatcher::new().map_err(|err| err.to_string())?,
        ))
    } else {
        None
    };

    let context = Arc::new(AppContext::new(
        Arc::new(SystemClock),
        Arc::clone(&events),
        Arc::new(SqliteProfileRepository::new(pool.clone())),
        Arc::new(SqliteSettingsRepository::new(pool.clone())),
    ));
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
            watcher: watcher.clone(),
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
    if let Ok(removed) = stats.purge_expired()
        && removed > 0
    {
        println!("cleared {removed} listening event(s) older than thirty days");
    }

    // Before anything else: the pointer left by the previous run decides who the
    // application is running as.
    let active = profiles.restore_active().map_err(|err| err.to_string())?;

    if command == Command::Watch {
        return watch(&library).map_err(|err| err.to_string());
    }

    if command == Command::Ui {
        // The audio device is opened here and nowhere else: a command line that
        // lists profiles has no business claiming the speakers.
        // One engine, shared: the equaliser and the transport are two things
        // asked of the same filters.
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

        // Built after the profile has been restored, because building it is
        // what restores that profile's queue.
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
        );

        // The library keeps itself current for as long as the window is open,
        // which is what "автоматическое отслеживание изменений файловой системы"
        // asks for and what `cadenza watch` could only do with a terminal open
        // beside it.
        if let Some(watcher) = watcher.as_ref() {
            watcher.set_handler(Box::new({
                let library = Arc::clone(&library);
                move |change| {
                    // A failure concerns one file, and tearing the watcher down over
                    // an unreadable download would cost the listener every other
                    // file. There is nowhere to report it to yet: the window is on
                    // another thread and this application writes no log. That is
                    // M16's "logs", and this is the first thing that will want one.
                    let _ = library.apply_change(&change);
                }
            }));
            library.watch_folders().map_err(|err| err.to_string())?;
        }

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

        // Before the pool goes: the worker holds a connection, and a thread
        // still decoding while the process exits is a file left open.
        analyser.stop();
        return outcome.map_err(|err| err.to_string());
    }

    dispatch(
        &command, &context, &profiles, &library, &playlists, &analysis, &stats, active,
    )
    .map_err(|err| err.to_string())
}

/// Watches the library folders until the listener stops it.
///
/// Blocks, unlike every other command. Until the interface exists in M6 this is
/// the only way to see the watcher work.
fn watch(library: &Arc<LibraryService>) -> Result<()> {
    let folders = library.folders()?;
    if folders.is_empty() {
        println!("no folders to watch — run: cadenza add-folder <path> [-r]");
        return Ok(());
    }

    let watcher = NotifyFileWatcher::new()?;

    let handler = Arc::clone(library);
    watcher.set_handler(Box::new(move |change| {
        // A failure here concerns one file. Reporting it and carrying on beats
        // tearing down the watcher over a single unreadable download.
        if let Err(err) = handler.apply_change(&change) {
            eprintln!("cadenza: {err}");
        } else {
            println!("  {change:?}");
        }
    }));

    for folder in &folders {
        watcher.watch(&folder.path, folder.include_subfolders)?;
        println!("watching {}", folder.path.display());
    }

    println!("\npress Enter to stop");
    let mut line = String::new();

    match io::stdin().read_line(&mut line) {
        // No terminal: started detached, or with input redirected. Reading gives
        // an immediate end of file, and exiting on that would stop the watcher
        // before it had seen anything — which is exactly what a background
        // watcher must not do.
        Ok(0) | Err(_) => {
            println!("no terminal attached — watching until this process is stopped");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Ok(_) => {}
    }

    // Dropping the watcher stops its thread and joins it, so nothing runs
    // against a half-dropped application.
    drop(watcher);
    Ok(())
}

/// Plays one file until it ends or the listener stops it.
///
/// Blocks, like `watch`. Until M6 there is no other way to hear the engine, and
/// the M5 definition of done — WAV, FLAC and MP3 play, pause works, seek works —
/// is a claim about a speaker that no test can make.
fn play(path: &Path) -> Result<()> {
    let probed = SymphoniaDecoder.probe(path)?;
    let engine = CpalAudioEngine::new()?;

    engine.load(path)?;
    engine.play()?;

    println!("output: {}", engine.description());
    println!(
        "file:   {} — {}, {} Hz, {} channels, {}",
        path.display(),
        probed.format,
        probed.properties.sample_rate,
        probed.properties.channels,
        probed.properties.duration
    );
    println!("\ncommands: p pause or resume, s <seconds> seek, v <0-100> volume, q quit");

    let mut line = String::new();
    loop {
        line.clear();
        match io::stdin().read_line(&mut line) {
            // No terminal: started detached or with input redirected. Play to
            // the end rather than exit on the immediate end of file.
            Ok(0) | Err(_) => {
                follow(&engine);
                break;
            }
            Ok(_) => {}
        }

        let mut words = line.split_whitespace();
        match words.next() {
            None => {}
            Some("q") => break,
            Some("p") => {
                if engine.state().is_playing() {
                    engine.pause()?;
                } else {
                    engine.play()?;
                }
            }
            Some("s") => match words.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(seconds) => engine.seek(PlaybackPosition::from_secs(seconds))?,
                None => println!("s takes a number of seconds, for example: s 30"),
            },
            Some("v") => match words.next().and_then(|value| value.parse::<f32>().ok()) {
                Some(percent) => engine.set_volume(Volume::clamped(percent / 100.0))?,
                None => println!("v takes 0 to 100, for example: v 40"),
            },
            Some(other) => println!("unknown command {other:?}"),
        }

        report(&engine);
    }

    engine.stop()?;
    if let Some(failure) = engine.failure() {
        println!("stopped: {failure}");
    }
    println!("underruns: {}", engine.underruns());
    Ok(())
}

/// Prints progress until the track ends. Used when there is no terminal to type
/// commands into.
fn follow(engine: &CpalAudioEngine) {
    while engine.state() != PlaybackState::Stopped {
        report(engine);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn report(engine: &CpalAudioEngine) {
    let state = match engine.state() {
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopped => "stopped",
    };
    println!(
        "  {state} {} / {}",
        engine.position().elapsed(),
        engine.duration()
    );
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    command: &Command,
    context: &AppContext,
    profiles: &ProfileService,
    library: &LibraryService,
    playlists: &PlaylistService,
    analysis: &AnalysisService,
    stats: &StatsService,
    active: Option<Profile>,
) -> Result<()> {
    match command {
        Command::Help | Command::Paths | Command::Watch | Command::Play(_) | Command::Ui => {
            unreachable!("handled before dispatch, because they need no profile or must block")
        }

        Command::Playlists => {
            let all = playlists.list()?;
            if all.is_empty() {
                println!("no playlists yet — run: cadenza playlist new <name>");
            }
            for summary in all {
                let noun = if summary.track_count == 1 {
                    "track"
                } else {
                    "tracks"
                };
                println!(
                    "  {} ({} {noun})",
                    summary.playlist.name, summary.track_count
                );
            }
        }

        Command::Playlist(action) => playlist(action, library, playlists)?,

        Command::Analyze { limit } => analyze(analysis, *limit)?,

        Command::Stats => report_listening(stats)?,

        Command::Status => report_status(profiles, active.as_ref())?,

        Command::Create(name) => {
            let created = profiles.create(name)?;
            println!("created profile {}", created.name);
            if active.is_none() {
                println!("started as {} — it is the only profile", created.name);
            }
        }

        Command::Switch(name) => {
            let target = find_by_name(profiles, name)?;
            let switched = profiles.switch_to(target.id)?;
            println!("now running as {}", switched.name);
        }

        Command::Delete { name, confirmed } => {
            let target = find_by_name(profiles, name)?;
            if !confirmed {
                println!(
                    "deleting {} removes its library, playlists, history and presets, \
                     and cannot be undone.\nrun: cadenza delete {name} --yes",
                    target.name
                );
                return Ok(());
            }
            profiles.delete(target.id)?;
            println!("deleted profile {}", target.name);
        }

        Command::History(enabled) => {
            let Some(profile) = active else {
                return Err(CoreError::NoActiveProfile);
            };
            let updated = profiles.set_history_enabled(profile.id, *enabled)?;
            println!(
                "history is now {} for {}",
                if updated.history_enabled { "on" } else { "off" },
                updated.name
            );
        }

        Command::Theme(dark) => {
            let Some(profile) = active else {
                return Err(CoreError::NoActiveProfile);
            };
            let mode = if *dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            let updated = profiles.set_theme(profile.id, mode)?;
            println!("{} now uses the {} theme", updated.name, updated.theme);
        }

        Command::Crossfade { enabled, seconds } => {
            let Some(profile) = active else {
                return Err(CoreError::NoActiveProfile);
            };
            let now = context.now();

            if let Some(seconds) = *seconds {
                let duration = CrossfadeDuration::new(DurationMs::from_secs(seconds))?;
                let millis = i64::try_from(duration.as_duration().as_millis()).map_err(|_| {
                    CoreError::invalid("crossfade", "the length does not fit a number")
                })?;
                context.settings.profile_set(
                    profile.id,
                    CROSSFADE_MS_KEY,
                    &SettingValue::Integer(millis),
                    now,
                )?;
            }

            context.settings.profile_set(
                profile.id,
                CROSSFADE_ENABLED_KEY,
                &SettingValue::Bool(*enabled),
                now,
            )?;

            // Read back rather than echoed: the length that matters is the one
            // stored, which is not always the one this command was given.
            let length = match context.settings.profile_get(profile.id, CROSSFADE_MS_KEY)? {
                Some(value) => {
                    DurationMs::from_millis(u64::try_from(value.as_integer()?).unwrap_or_default())
                }
                None => CrossfadeDuration::DEFAULT.as_duration(),
            };

            if *enabled {
                println!(
                    "{} now crossfades ordinary tracks over {}",
                    profile.name, length
                );
            } else {
                println!("{} no longer crossfades", profile.name);
            }
            println!(
                "playlists and radio stay gapless either way; a window already open picks this up when it restarts"
            );
        }

        Command::Folders => {
            let folders = library.folders()?;
            if folders.is_empty() {
                println!("no folders yet — run: cadenza add-folder <path> [-r]");
            }
            for folder in folders {
                println!(
                    "  {} {}{}",
                    if folder.enabled { "*" } else { " " },
                    folder.path.display(),
                    if folder.include_subfolders {
                        " (with subfolders)"
                    } else {
                        ""
                    }
                );
            }
        }

        Command::AddFolder { path, recursive } => {
            // Added and taken in, as one act. Pointing at a folder is saying
            // "here is my music"; a separate scan afterwards asks for it twice.
            let folder = library.add_folder(Path::new(path), *recursive)?;
            let report = library.adopt_folder(&folder)?;
            println!("watching {}", folder.path.display());
            println!(
                "{} file(s) seen: {} added, {} updated, {} unchanged",
                report.seen, report.added, report.updated, report.unchanged
            );
        }

        Command::Scan => {
            let report = library.scan_all()?;
            println!(
                "{} file(s) seen: {} added, {} updated, {} unchanged",
                report.seen, report.added, report.updated, report.unchanged
            );
            if report.duplicates > 0 || report.failed > 0 {
                println!(
                    "{} duplicate(s) and {} unreadable file(s) need a decision — \
                     run: cadenza reviews",
                    report.duplicates, report.failed
                );
            }
        }

        Command::Tracks => {
            let tracks = library.tracks()?;
            if tracks.is_empty() {
                println!("the library is empty — run: cadenza scan");
            }
            // Numbered, because `cadenza genre` needs a way to name a track and
            // nobody is going to type a UUID at a command line.
            for (position, track) in tracks.iter().enumerate() {
                let genres = library.genres_of(track.media_file_id)?;
                let names = genres
                    .iter()
                    .map(|genre| genre.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");

                if names.is_empty() {
                    println!("  {} {}", position + 1, track.title);
                } else {
                    println!("  {} {} [{names}]", position + 1, track.title);
                }
            }
        }

        Command::Genre {
            index,
            names,
            reset,
        } => {
            let tracks = library.tracks()?;
            let track = tracks
                .get(index.wrapping_sub(1))
                .ok_or_else(|| CoreError::not_found("track number", index))?;

            if *reset {
                library.reset_genres(track.media_file_id)?;
            } else {
                library.set_genres(track.media_file_id, names)?;
            }

            let genres = library.genres_of(track.media_file_id)?;
            let listed = genres
                .iter()
                .map(|genre| genre.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            println!(
                "{} is filed under {} for this profile only",
                track.title,
                if listed.is_empty() {
                    "nothing".to_owned()
                } else {
                    listed
                }
            );
        }

        Command::Reviews => {
            let pending = library.pending_reviews()?;
            if pending.is_empty() {
                println!("nothing is waiting for a decision");
            }
            for entry in pending {
                println!("  {} {}", entry.reason.as_str(), entry.media_file_id);
            }
        }
    }

    Ok(())
}

/// Everything under `cadenza playlist`.
fn playlist(
    action: &PlaylistCommand,
    library: &LibraryService,
    playlists: &PlaylistService,
) -> Result<()> {
    match action {
        PlaylistCommand::New(name) => {
            let created = playlists.create(name)?;
            println!("created playlist {}", created.name);
            println!("run: cadenza playlist add {} <track number>", created.name);
        }

        PlaylistCommand::Show(name) => {
            let found = find_playlist(playlists, name)?;
            let tracks = playlists.tracks_of(found.id)?;
            if tracks.is_empty() {
                println!("{} is empty", found.name);
            }
            for (position, track) in tracks.iter().enumerate() {
                println!("  {} {}", position + 1, track.title);
            }
        }

        PlaylistCommand::Rename { from, to } => {
            let found = find_playlist(playlists, from)?;
            let renamed = playlists.rename(found.id, to)?;
            println!("{from} is now {}", renamed.name);
        }

        PlaylistCommand::Delete { name, confirmed } => {
            let found = find_playlist(playlists, name)?;
            if !confirmed {
                println!(
                    "deleting {} removes the list, not the tracks in it.\n\
                     run: cadenza playlist delete {name} --yes",
                    found.name
                );
                return Ok(());
            }
            playlists.delete(found.id)?;
            println!("deleted playlist {}", found.name);
        }

        PlaylistCommand::Add { name, track } => {
            let found = find_playlist(playlists, name)?;
            let tracks = library.tracks()?;
            let chosen = tracks
                .get(track.wrapping_sub(1))
                .ok_or_else(|| CoreError::not_found("track number", track))?;

            playlists.add_track(found.id, chosen.media_file_id)?;
            println!("added {} to {}", chosen.title, found.name);
        }

        PlaylistCommand::Remove { name, entry } => {
            let found = find_playlist(playlists, name)?;
            playlists.remove_at(found.id, entry.wrapping_sub(1))?;
            println!("removed entry {entry} from {}", found.name);
        }

        PlaylistCommand::Move { name, from, to } => {
            let found = find_playlist(playlists, name)?;
            playlists.move_entry(found.id, from.wrapping_sub(1), to.wrapping_sub(1))?;
            println!("moved entry {from} to {to} in {}", found.name);
        }
    }

    Ok(())
}

/// Looks a playlist up the way a person refers to one.
fn find_playlist(
    playlists: &PlaylistService,
    name: &str,
) -> Result<cadenza_core::domain::playlist::Playlist> {
    playlists
        .list()?
        .into_iter()
        .map(|summary| summary.playlist)
        .find(|playlist| playlist.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| CoreError::not_found("playlist", name))
}

/// Looks a profile up the way a person refers to one.
///
/// Names are unique per installation, so this is unambiguous — and nobody is
/// going to type a UUID at a command line.
fn find_by_name(profiles: &ProfileService, name: &str) -> Result<Profile> {
    profiles
        .list()?
        .into_iter()
        .find(|profile| profile.name.as_str().eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| CoreError::not_found("profile", name))
}

fn report_status(profiles: &ProfileService, active: Option<&Profile>) -> Result<()> {
    let all = profiles.list()?;

    if all.is_empty() {
        println!("no profiles yet — run: cadenza create <name>");
        return Ok(());
    }

    match active {
        Some(profile) => println!(
            "running as {} (history {}, theme {})",
            profile.name,
            if profile.history_enabled { "on" } else { "off" },
            profile.theme
        ),
        None => println!("no active profile — run: cadenza switch <name>"),
    }

    println!("\nprofiles:");
    for profile in &all {
        let marker = if active.is_some_and(|current| current.id == profile.id) {
            "*"
        } else {
            " "
        };
        println!("  {marker} {}", profile.name);
    }

    Ok(())
}

/// Works through the analysis queue, saying what each file turned out to be.
///
/// The same steps the background worker takes, without the resting between
/// them: this is where the numbers can be looked at, and where the cost of
/// producing them can be timed.
fn analyze(analysis: &AnalysisService, limit: Option<usize>) -> Result<()> {
    let queued = analysis.top_up()?;
    let progress = analysis.progress()?;

    if progress.is_settled() && queued == 0 {
        println!(
            "nothing to analyse — {} file(s) already done by {}",
            progress.analysed,
            analysis.extractor_version()
        );
        return Ok(());
    }

    let wanted = limit.unwrap_or(usize::MAX);
    let started = std::time::Instant::now();
    let mut done = 0usize;

    while done < wanted {
        let at = std::time::Instant::now();
        let Some(media_file_id) = analysis.run_next()? else {
            // The batch is finished; there may be more of the library behind it.
            if analysis.top_up()? == 0 {
                break;
            }
            continue;
        };
        done += 1;

        match analysis.features(media_file_id)? {
            Some(features) => println!(
                "  {:>5.1}s  {:>7}  {:>7}  energy {:.2}  dance {:.2}  bright {:.2}",
                at.elapsed().as_secs_f32(),
                features
                    .bpm
                    .map_or_else(|| "-".to_owned(), |bpm| format!("{:.0} bpm", bpm.as_f32())),
                features
                    .key
                    .map_or_else(|| "-".to_owned(), |key| key.to_string()),
                features.energy,
                features.danceability,
                features.spectral_centroid,
            ),
            // Analysed and no features: the file was claimed, tried and failed,
            // which the job now carries as an error and an attempt.
            None => println!(
                "  {:>5.1}s  could not be analysed",
                at.elapsed().as_secs_f32()
            ),
        }
    }

    let after = analysis.progress()?;
    println!(
        "{done} file(s) in {:.1}s — {} analysed, {} still waiting",
        started.elapsed().as_secs_f32(),
        after.analysed,
        after.pending
    );
    Ok(())
}

/// Prints the month the listener has had.
fn report_listening(stats: &StatsService) -> Result<()> {
    let report = stats.report()?;

    if !report.keeping {
        println!("history is off for this profile — nothing is being recorded");
        println!("turn it on with: cadenza history on");
        return Ok(());
    }

    let summary = report.summary;
    if summary.started == 0 {
        println!("nothing played in the last {} days", report.days);
        return Ok(());
    }

    let minutes = summary.listened.as_millis() / 60_000;
    println!(
        "the last {} days: {} listen(s) of {} track(s), {} minute(s) heard",
        report.days, summary.started, summary.tracks, minutes
    );
    println!(
        "  {} played through, {} skipped early ({:.0}%)",
        summary.completed,
        summary.skipped,
        summary.skip_rate() * 100.0
    );

    if report.top.is_empty() {
        return Ok(());
    }

    println!(
        "
played through most:"
    );
    for (track, plays) in &report.top {
        let noun = if *plays == 1 { "time" } else { "times" };
        println!("  {plays:>3} {noun}  {}", track.title);
    }
    Ok(())
}

fn report_paths(paths: &AppPaths) {
    use io::Write as _;

    let mut out = io::stdout().lock();
    let _ = writeln!(out, "database:  {}", paths.database().display());
    let _ = writeln!(out, "settings:  {}", paths.settings_file().display());
    let _ = writeln!(out, "artwork:   {}", paths.artwork_cache_dir().display());
    let _ = writeln!(out, "analysis:  {}", paths.analysis_temp_dir().display());
    let _ = writeln!(out, "log:       {}", paths.log_file().display());
}
