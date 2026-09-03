//! Bringing a track down from a link somebody pasted.
//!
//! Cadenza is a *local* player: the music is files on your disk and nothing it
//! does depends on a service being up. That is not the same as a program that
//! refuses to reach the network at all, and the difference is the whole shape
//! of this port.
//!
//! Nothing here opens a socket. Cadenza's own code has no HTTP client, no TLS,
//! no parser for anybody's website. What it has is the ability to run a program
//! the listener installed, once, because they pasted a link and pressed a
//! button — and then to take the file that program left behind and treat it
//! exactly like a file they had copied in themselves. Nothing runs in the
//! background, nothing is sent anywhere, and a machine with no network is a
//! machine where every other part of Cadenza works unchanged.
//!
//! Two consequences worth knowing before reading the adapter. Keeping the
//! extraction outside means it stays current without us: the sites change, the
//! tool is updated by the people who watch them, and a button here does not
//! quietly stop working between our releases. And it means the installer ships
//! no such tool, so what Cadenza distributes is a player.

use std::path::{Path, PathBuf};

use crate::Result;

/// A program this needs and the machine does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingTool {
    /// What to install, as it is called.
    pub name: String,
    /// What it is for, in a few words the listener can act on.
    pub reason: String,
}

/// Running somebody else's downloader on the listener's behalf.
pub trait FetchPort: Send + Sync {
    /// Which of the programs this needs are not on this machine.
    ///
    /// Asked before anything is attempted, so that "you need to install
    /// yt-dlp" arrives instead of a failure ten seconds into a download.
    fn missing(&self) -> Vec<MissingTool>;

    /// Brings the audio at `link` into `into` as one mp3, and says where.
    ///
    /// `progress` is called with whole percentages as they arrive. It runs on
    /// whatever thread called this, which is never the one drawing the window.
    fn fetch(&self, link: &str, into: &Path, progress: &dyn Fn(u8)) -> Result<PathBuf>;
}
