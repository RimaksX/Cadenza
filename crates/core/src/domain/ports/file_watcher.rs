//! Noticing that files changed underneath us.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::domain::value_objects::DurationMs;

/// How long to wait for a burst of filesystem events to settle.
///
/// A calibration knob (PROJECT_MASTER 3.6 requires debouncing). Copying an album
/// into a watched folder produces one event per file per write; without a delay
/// the scanner would start on half-written files. Too long and the library feels
/// unresponsive to a single drag-and-drop.
pub const DEBOUNCE: DurationMs = DurationMs::from_millis(750);

/// Something happened to a watched path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    /// A new file appeared.
    Created(PathBuf),
    /// An existing file was written to.
    Modified(PathBuf),
    /// A file disappeared.
    Removed(PathBuf),
    /// A file moved. Handled as a move rather than a delete plus create, so the
    /// library keeps the track's identity, history and playlist membership.
    Renamed {
        /// Where it was.
        from: PathBuf,
        /// Where it is now.
        to: PathBuf,
    },
}

/// A callback for debounced filesystem changes.
pub type FileChangeHandler = Box<dyn Fn(FileChange) + Send + Sync>;

/// Watches library folders for changes.
pub trait FileWatcherPort: Send + Sync {
    /// Starts watching a folder. Events are delivered after [`DEBOUNCE`].
    fn watch(&self, path: &Path, recursive: bool) -> Result<()>;

    /// Stops watching a folder.
    fn unwatch(&self, path: &Path) -> Result<()>;

    /// Installs the handler that receives change notifications.
    fn set_handler(&self, handler: FileChangeHandler);
}
