//! Bringing a track down from a link somebody pasted.
//!
//! **Nothing here opens a socket.** Cadenza has no HTTP client, no TLS and no
//! parser for anybody's website. It runs a program the listener installed,
//! once, because they pasted a link and pressed a button, then treats the file
//! that program left behind like any other file they copied in. Nothing runs in
//! the background, and a machine with no network is one where every other part
//! of Cadenza works unchanged.
//!
//! Keeping the extraction outside also keeps it current without us — the sites
//! change and the tool is updated by people who watch them — and it means the
//! installer ships no such tool, so what Cadenza distributes is a player.

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

/// One track a list names, as the service that named it calls it.
///
/// Title and artist rather than a file name: both come from the same metadata
/// the tags were written from, so this matches what the library knows even
/// when the file on disk was named differently or renamed since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTrack {
    pub title: String,
    pub artist: String,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchedTracks {
    /// The mp3s, in the order the playlist held them.
    pub files: Vec<PathBuf>,
    /// Every track the list names, whether it arrived just now or was already
    /// here.
    ///
    /// A playlist holds the *list*, and a second fetch of the same address
    /// fetches almost nothing — everything is already on the disk. Without this
    /// the playlist made from that fetch would hold the two tracks that
    /// happened to be new.
    pub listed: Vec<ListedTrack>,
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
    /// Which of the programs *this link* needs are not on this machine.
    ///
    /// Asked before anything is attempted, so that "you need to install
    /// yt-dlp" arrives instead of a failure ten seconds into a download.
    ///
    /// Per link, because they do not all need the same things: a Spotify
    /// address needs the program that reads its names and finds the recording,
    /// and a listener who only ever pastes YouTube links should never be told
    /// to install it.
    fn missing_for(&self, link: &str) -> Vec<MissingTool>;

    /// Installs whatever [`Self::missing_for`] reported for this link, and says
    /// what is still missing afterwards.
    ///
    /// Through the machine's own package manager, which is the posture of
    /// everything here: Cadenza opens no connection, it runs a program already
    /// on the machine. What it runs is reported line by line through `said`,
    /// because installing something on somebody's computer is not a thing to do
    /// behind a spinner.
    fn install(&self, link: &str, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>>;

    /// Brings the programs up to date, and says what they said.
    ///
    /// The downloader through its own updater, and anything installed through
    /// the machine's package manager through that.
    ///
    /// Its own rather than the package manager's, measured rather than assumed:
    /// the package in `winget` on the machine this was written on was six weeks
    /// behind the copy that was actually running, because the copy had already
    /// updated itself.
    fn update(&self, said: &dyn Fn(&str)) -> Result<String>;

    /// Brings audio from `link` into `into` as mp3s, and says where they went.
    ///
    /// One file for [`FetchWhat::OneTrack`], as many as the playlist held for
    /// [`FetchWhat::WholePlaylist`] — including none, if every one of them
    /// failed, which is not an error here: the caller is told what landed.
    ///
    /// `progress` is called as reports arrive and `stop` is asked between them,
    /// so a playlist somebody changed their mind about ends when they say so.
    /// `have` is asked before each track of a list and one it recognises is not
    /// fetched at all — what counts as *already here* is the library's decision,
    /// not this port's.
    ///
    /// All three run on whatever thread called this, which is never the one
    /// drawing the window.
    fn fetch(
        &self,
        link: &str,
        into: &Path,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
        have: &dyn Fn(&ListedTrack) -> bool,
    ) -> Result<FetchedTracks>;
}
