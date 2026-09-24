//! Asking the listener for a folder or a file.

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

    /// Opens the chooser for one file of a kind this application reads.
    ///
    /// Provided as "nothing chosen" so that a chooser built for a test that
    /// never asks for one need not say so; the real one answers.
    fn pick_file(&self, title: &str, kind: FileKind) -> Result<Option<PathBuf>> {
        let _ = (title, kind);
        Ok(None)
    }

    /// Asks where to save a file of a kind this application writes, offering
    /// `suggested` as its name. The system's own dialog asks before replacing
    /// a file that is there already, so an answer is a place to write.
    fn save_file(&self, title: &str, kind: FileKind, suggested: &str) -> Result<Option<PathBuf>> {
        let _ = (title, kind, suggested);
        Ok(None)
    }
}

/// The files the chooser offers, each with the extensions it is known by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A list of tracks: `.m3u8`, or the older `.m3u`.
    Playlist,
    /// A copy of everything Cadenza keeps.
    Backup,
}

impl FileKind {
    /// The extensions, the one written first.
    #[must_use]
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Playlist => &["m3u8", "m3u"],
            Self::Backup => &["cadenza"],
        }
    }
}
