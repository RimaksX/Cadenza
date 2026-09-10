//! Where Cadenza keeps its files.
//!
//! The layout is fixed. Configuration and the database
//! go under the roaming profile so they follow the user between machines on a
//! domain; caches and logs go under the local one, because they are large,
//! regenerable and nobody wants them synchronised.

use std::fs;
use std::path::{Path, PathBuf};

use cadenza_core::{CoreError, Result};
use directories::BaseDirs;

/// The folder name under both roots.
pub const APP_DIRECTORY: &str = "Cadenza";

/// Resolved locations for everything Cadenza writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    config: PathBuf,
    local: PathBuf,
}

impl AppPaths {
    /// Resolves the paths from the operating system.
    ///
    /// Uses `BaseDirs` rather than `ProjectDirs`: the latter would produce
    /// `%APPDATA%/Cadenza/config/app.db`, and section 6 asks for
    /// `%APPDATA%/Cadenza/app.db`. Appending the application folder by hand is
    /// the difference.
    pub fn resolve() -> Result<Self> {
        let base = BaseDirs::new().ok_or_else(|| {
            CoreError::FileSystem(
                "could not determine the user's home directory; \
                 Cadenza has nowhere to store its data"
                    .to_owned(),
            )
        })?;

        Ok(Self::with_roots(
            base.config_dir().join(APP_DIRECTORY),
            base.data_local_dir().join(APP_DIRECTORY),
        ))
    }

    /// Builds the layout over explicit roots. For tests, which must not write
    /// into the real user profile.
    #[must_use]
    pub fn with_roots(config: PathBuf, local: PathBuf) -> Self {
        Self { config, local }
    }

    /// `%APPDATA%/Cadenza`.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    /// `%LOCALAPPDATA%/Cadenza`.
    #[must_use]
    pub fn local_dir(&self) -> &Path {
        &self.local
    }

    /// The SQLite database.
    #[must_use]
    pub fn database(&self) -> PathBuf {
        self.config.join("app.db")
    }

    /// The settings file kept outside the database, for anything needed before
    /// it can be opened.
    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// What has already been brought down from a link.
    ///
    /// One line per track, written by `yt-dlp` and read by it: it is the file
    /// that makes a second press on a playlist carry on where the first was
    /// stopped rather than fetch the same forty tracks again. Kept beside the
    /// cache rather than in the music folder, because it is a record of what
    /// this machine did and not a thing to listen to.
    #[must_use]
    pub fn fetch_archive_file(&self) -> PathBuf {
        self.local.join("fetched.txt")
    }

    /// Logging configuration.
    #[must_use]
    pub fn log_config_file(&self) -> PathBuf {
        self.config.join("app.log.config.json")
    }

    /// Cached cover images.
    #[must_use]
    pub fn artwork_cache_dir(&self) -> PathBuf {
        self.local.join("cache").join("artwork")
    }

    /// Scratch space for the analysis worker.
    #[must_use]
    pub fn analysis_temp_dir(&self) -> PathBuf {
        self.local.join("cache").join("analysis_tmp")
    }

    /// The current log.
    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.local.join("logs").join("app.log")
    }

    /// The previous log, kept across one rotation.
    #[must_use]
    pub fn previous_log_file(&self) -> PathBuf {
        self.local.join("logs").join("app.old.log")
    }

    /// Creates every directory Cadenza writes into.
    ///
    /// Called once at startup so that later failures are about the operation
    /// that failed rather than about a missing folder.
    pub fn ensure_directories(&self) -> Result<()> {
        for directory in [
            self.config.clone(),
            self.artwork_cache_dir(),
            self.analysis_temp_dir(),
            self.local.join("logs"),
        ] {
            fs::create_dir_all(&directory).map_err(|err| {
                CoreError::FileSystem(format!("could not create {}: {err}", directory.display()))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AppPaths;

    fn paths() -> AppPaths {
        AppPaths::with_roots(
            PathBuf::from("C:/Users/tester/AppData/Roaming/Cadenza"),
            PathBuf::from("C:/Users/tester/AppData/Local/Cadenza"),
        )
    }

    #[test]
    fn the_layout_matches_the_specification() {
        let paths = paths();

        assert!(paths.database().ends_with("Cadenza/app.db"));
        assert!(paths.settings_file().ends_with("Cadenza/settings.json"));
        assert!(
            paths
                .log_config_file()
                .ends_with("Cadenza/app.log.config.json")
        );
        assert!(paths.artwork_cache_dir().ends_with("cache/artwork"));
        assert!(paths.analysis_temp_dir().ends_with("cache/analysis_tmp"));
        assert!(paths.log_file().ends_with("logs/app.log"));
        assert!(paths.previous_log_file().ends_with("logs/app.old.log"));
    }

    #[test]
    fn the_database_is_not_in_a_local_cache() {
        let paths = paths();
        assert!(
            paths.database().starts_with(paths.config_dir()),
            "the database must live beside the configuration, not in the cache"
        );
        assert!(paths.artwork_cache_dir().starts_with(paths.local_dir()));
    }

    #[test]
    fn creating_directories_is_repeatable() {
        let root = std::env::temp_dir().join(format!("cadenza-paths-{}", std::process::id()));
        let paths = AppPaths::with_roots(root.join("roaming"), root.join("local"));

        paths.ensure_directories().expect("first run");
        paths.ensure_directories().expect("second run is a no-op");

        assert!(paths.artwork_cache_dir().is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }
}
