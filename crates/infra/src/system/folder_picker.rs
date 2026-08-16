//! The system's folder chooser.

use std::path::{Path, PathBuf};

use directories::UserDirs;

use cadenza_core::Result;
use cadenza_core::domain::ports::folder_picker::FolderPickerPort;

/// Opens the chooser Windows already has.
///
/// `rfd` is a thin wrapper over `IFileDialog`, which is COM and would mean
/// `unsafe` in a crate that forbids it. What it buys is that one call: no
/// runtime, no toolkit, and on this platform no second window system.
pub struct SystemFolderPicker;

impl FolderPickerPort for SystemFolderPicker {
    fn pick_folder(&self, title: &str) -> Result<Option<PathBuf>> {
        // No parent handle. The window that asked lives in `cadenza-ui`, which
        // this crate cannot see and must not; the dialog still comes up in
        // front, because the process asking is the foreground one. What it
        // loses is being *owned* by that window, which shows only if somebody
        // clicks behind it. A handle can be threaded through the port the day
        // that matters.
        Ok(rfd::FileDialog::new().set_title(title).pick_folder())
    }

    fn suggested_music_folder(&self) -> Option<PathBuf> {
        // The system's own music folder with a room of ours inside it, rather
        // than the whole thing: somebody who accepts the suggestion is saying
        // "put a library here", not "read everything I have ever downloaded".
        UserDirs::new().map(|dirs| {
            dirs.audio_dir()
                .map_or_else(|| dirs.home_dir().join("Music"), Path::to_path_buf)
                .join("Cadenza")
        })
    }
}
