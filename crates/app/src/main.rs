//! Cadenza composition root.
//!
//! The only place that constructs concrete infrastructure and hands it to the
//! application layer as port implementations (PROJECT_MASTER 4.4).
//!
//! Wiring lives here rather than in a `wiring.rs` of its own: there are four
//! dependencies. It gets its own file when it earns one.

#![forbid(unsafe_code)]

mod cli;

use std::process::ExitCode;
use std::sync::Arc;
use std::{env, io};

use cadenza_core::application::{AppContext, ProfileService};
use cadenza_core::domain::profile::Profile;
use cadenza_core::{CoreError, Result};
use cadenza_infra::db;
use cadenza_infra::db::repositories::{SqliteProfileRepository, SqliteSettingsRepository};
use cadenza_infra::events::InProcessEventBus;
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
        Arc::new(SqliteSettingsRepository::new(pool)),
    ));
    let profiles = ProfileService::new(Arc::clone(&context));

    // Before anything else: the pointer left by the previous run decides who the
    // application is running as.
    let active = profiles.restore_active().map_err(|err| err.to_string())?;

    dispatch(&command, &profiles, active).map_err(|err| err.to_string())
}

fn dispatch(command: &Command, profiles: &ProfileService, active: Option<Profile>) -> Result<()> {
    match command {
        Command::Help | Command::Paths => unreachable!("handled before the database is opened"),

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
