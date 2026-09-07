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
    /// What to type to get it, exactly.
    ///
    /// The whole command and not the program's name, because the name is not
    /// enough: `winget install yt-dlp` matches both the package and something
    /// else in the Microsoft Store and refuses to choose, which is where
    /// somebody told to "install yt-dlp" actually ends up.
    pub install: String,
}

/// What a link is being asked for.
///
/// A YouTube address often carries a playlist on the end of it, and the two
/// readings of the same link are worth different things: somebody who pressed
/// GET wants the track they were looking at, and somebody who pressed PLAYLIST
/// wants the forty behind it. Neither can be guessed from the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchWhat {
    /// The one track the link points at, whatever else it carries.
    OneTrack,
    /// Everything in the playlist the link carries.
    WholePlaylist,
}

/// How far a fetch has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FetchProgress {
    /// Whole percentages, of the file being downloaded now.
    pub percent: u8,
    /// Which track of how many, where more than one is coming.
    ///
    /// A playlist is not one download with a percentage; it is forty of them,
    /// and "3 of 40" is the only number that answers "how long is this going
    /// to take".
    pub item: Option<(u32, u32)>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchedTracks {
    /// The mp3s, in the order the playlist held them.
    pub files: Vec<PathBuf>,
    /// What the playlist is called, where a playlist is what was asked for.
    ///
    /// The downloader knows it — it prints it before the first track — and it
    /// is the only name anybody would recognise. The album tag is not it: half
    /// a mixtape carries no album at all, and the other half carries forty
    /// different ones.
    pub playlist: Option<String>,
}

/// Running somebody else's downloader on the listener's behalf.
pub trait FetchPort: Send + Sync {
    /// Which of the programs this needs are not on this machine.
    ///
    /// Asked before anything is attempted, so that "you need to install
    /// yt-dlp" arrives instead of a failure ten seconds into a download.
    fn missing(&self) -> Vec<MissingTool>;

    /// Brings audio from `link` into `into` as mp3s, and says where they went.
    ///
    /// One file for [`FetchWhat::OneTrack`] and as many as the playlist held
    /// for [`FetchWhat::WholePlaylist`] — including none, if every one of them
    /// failed, which is not an error here: the caller is told what landed and
    /// says so.
    ///
    /// `progress` is called as reports arrive, and `stop` is asked between
    /// them: a playlist somebody changed their mind about has to end when they
    /// say so and not when it finishes. Both run on whatever thread called
    /// this, which is never the one drawing the window.
    /// Installs whatever [`Self::missing`] reported, and says what is still
    /// missing afterwards.
    ///
    /// Through the machine's own package manager, which is the same posture as
    /// everything else here: Cadenza opens no connection, it runs a program
    /// that is already on the machine and lets that program do its job. What
    /// it runs is reported line by line through `said`, because installing
    /// something on somebody's computer is not a thing to do behind a spinner
    /// (`MASTER_ISSUES` 96).
    fn install(&self, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>>;

    /// Brings the downloader up to date with its own updater, and says what it
    /// said.
    ///
    /// Its own rather than the package manager's, measured rather than assumed:
    /// the package in `winget` on the machine this was written on was six weeks
    /// behind the copy that was actually running, because the copy had already
    /// updated itself.
    fn update(&self, said: &dyn Fn(&str)) -> Result<String>;

    fn fetch(
        &self,
        link: &str,
        into: &Path,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> Result<FetchedTracks>;
}
