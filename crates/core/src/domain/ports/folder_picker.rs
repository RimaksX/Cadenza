//! Asking the listener which folder holds their music.

use std::path::PathBuf;

use crate::Result;

/// The system's own folder chooser.
///
/// A port rather than a call, for the reason every port here exists: the window
/// may not reach the operating system, and the application layer may not know
/// which one it is running on. What crosses this boundary is a path or nothing.
///
/// It **blocks** until the listener answers, which is what a modal dialog is.
/// The caller is the interface thread and knows that; nothing else may call it.
pub trait FolderPickerPort: Send + Sync {
    /// Opens the chooser and returns what was chosen.
    ///
    /// `None` means the listener closed it without choosing, which is an answer
    /// rather than a failure. An error means the chooser could not be opened at
    /// all.
    fn pick_folder(&self, title: &str) -> Result<Option<PathBuf>>;

    /// Opens the chooser for one image file.
    ///
    /// The same contract: `None` is an answer. What comes back is a path and
    /// not bytes, because reading it is the caller's job and the caller is the
    /// one that knows what it will accept.
    fn pick_image(&self, title: &str) -> Result<Option<PathBuf>>;

    /// Where this machine keeps music, if it has an opinion.
    ///
    /// Windows and every desktop like it have a folder for this, and somebody
    /// with no library yet should not have to invent one. What is returned is
    /// a *suggestion*: nothing is created until it is accepted.
    fn suggested_music_folder(&self) -> Option<PathBuf>;
}
