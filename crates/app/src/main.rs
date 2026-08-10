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

use cadenza_core::application::services::{LibraryPorts, LibraryService};
use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::playback::PlaybackState;
use cadenza_core::domain::ports::audio_engine::AudioEnginePort;
use cadenza_core::domain::ports::decoder::DecoderPort;
use cadenza_core::domain::ports::file_watcher::FileWatcherPort;
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::value_objects::{PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use cadenza_infra::audio::{CpalAudioEngine, SymphoniaDecoder};
use cadenza_infra::db;
use cadenza_infra::db::repositories::{
    SqliteAlbumRepository, SqliteArtistRepository, SqliteGenreRepository,
    SqliteImportReviewRepository, SqliteMediaFileRepository, SqliteProfileRepository,
    SqliteSettingsRepository, SqliteTrackRepository,
};
use cadenza_infra::events::InProcessEventBus;
use cadenza_infra::library::{LocalFileSystem, NotifyFileWatcher};
use cadenza_infra::metadata::{FileArtworkCache, LoftyMetadataReader};
use cadenza_infra::system::{AppPaths, SystemClock};

use cli::Command;

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

    let context = Arc::new(AppContext::new(
        Arc::new(SystemClock),
        Arc::new(InProcessEventBus::new()),
        Arc::new(SqliteProfileRepository::new(pool.clone())),
        Arc::new(SqliteSettingsRepository::new(pool.clone())),
    ));
    let profiles = ProfileService::new(Arc::clone(&context));

    let library = Arc::new(LibraryService::new(
        Arc::clone(&context),
        LibraryPorts {
            files: Arc::new(LocalFileSystem),
            metadata: Arc::new(LoftyMetadataReader),
            artwork: Arc::new(
                FileArtworkCache::new(paths.artwork_cache_dir()).map_err(|err| err.to_string())?,
            ),
            media_files: Arc::new(SqliteMediaFileRepository::new(pool.clone())),
            tracks: Arc::new(SqliteTrackRepository::new(pool.clone())),
            artists: Arc::new(SqliteArtistRepository::new(pool.clone())),
            albums: Arc::new(SqliteAlbumRepository::new(pool.clone())),
            genres: Arc::new(SqliteGenreRepository::new(pool.clone())),
            reviews: Arc::new(SqliteImportReviewRepository::new(pool)),
        },
    ));

    // Before anything else: the pointer left by the previous run decides who the
    // application is running as.
    let active = profiles.restore_active().map_err(|err| err.to_string())?;

    if command == Command::Watch {
        return watch(&library).map_err(|err| err.to_string());
    }

    dispatch(&command, &profiles, &library, active).map_err(|err| err.to_string())
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

fn dispatch(
    command: &Command,
    profiles: &ProfileService,
    library: &LibraryService,
    active: Option<Profile>,
) -> Result<()> {
    match command {
        Command::Help | Command::Paths | Command::Watch | Command::Play(_) => {
            unreachable!("handled before dispatch, because they need no profile or must block")
        }

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
            let folder = library.add_folder(Path::new(path), *recursive)?;
            println!("watching {}", folder.path.display());
            println!("run: cadenza scan");
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

fn report_paths(paths: &AppPaths) {
    use io::Write as _;

    let mut out = io::stdout().lock();
    let _ = writeln!(out, "database:  {}", paths.database().display());
    let _ = writeln!(out, "settings:  {}", paths.settings_file().display());
    let _ = writeln!(out, "artwork:   {}", paths.artwork_cache_dir().display());
    let _ = writeln!(out, "analysis:  {}", paths.analysis_temp_dir().display());
    let _ = writeln!(out, "log:       {}", paths.log_file().display());
}
