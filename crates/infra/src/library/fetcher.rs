//! Running the downloader the listener installed.
//!
//! Cadenza opens no connection of its own. This starts `yt-dlp`, reads what it
//! says about its own progress, and takes the file it leaves behind — the same
//! arrangement as the folder picker, where the work belongs to a program that
//! already exists and this crate only knows how to ask.
//!
//! Neither program is shipped with Cadenza. That is a deliberate three-way
//! saving: the installer stays a player at thirty megabytes rather than a
//! hundred and fifty, the licences of two more programs stay theirs, and — the
//! one that matters in a year — the extraction stays current without us. Sites
//! change; the people who maintain `yt-dlp` follow them within days, and a copy
//! frozen into our installer would be a button that stopped working between our
//! releases with nobody able to fix it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cadenza_core::domain::policies::link_policy::{LinkHandler, handler_for};
use cadenza_core::domain::ports::fetcher::{
    FetchPort, FetchProgress, FetchWhat, FetchedTracks, ListedTrack, MissingTool,
};
use cadenza_core::{CoreError, Result};

/// The downloader itself.
const DOWNLOADER: &str = "yt-dlp";

/// What turns whatever came down into an mp3.
const CONVERTER: &str = "ffmpeg";

/// The package manager that comes with Windows.
///
/// Cadenza installs nothing itself. It asks the thing that installs software on
/// this machine to install software on this machine, which is the same posture
/// as handing a link to `yt-dlp`: the program that does the work is the
/// program whose job it is.
const PACKAGES: &str = "winget";

/// What reads a Spotify link's names and finds the recording behind them.
///
/// It does not take anything out of Spotify — nothing does — and it says so
/// about itself. What it is for is the names: a track fetched this way carries
/// its title, artist and album rather than the title of a video
/// (`MASTER_ISSUES` 99).
const MATCHER: &str = "spotdl";

/// Python's package manager, which is the only place `spotdl` comes from.
const PYTHON_PACKAGES: &str = "pip";

/// Where the matcher looks for a recording, in order.
///
/// Its own default is YouTube Music alone, and a slice of the failures above
/// are not downloads at all but *searches*: "YouTube Music returned no usable
/// results for … after 3 attempts". Plain YouTube behind it is the answer to
/// those, and it is the same place the other button fetches from anyway.
const LOOK_IN: [&str; 2] = ["youtube-music", "youtube"];

/// Where a program is installed from.
#[derive(Clone, Copy)]
enum Source {
    /// The machine's own package manager, by exact identifier.
    ///
    /// Exact on purpose: `winget install yt-dlp` matches both the package and
    /// something else in the Microsoft Store and refuses to choose, which is
    /// where somebody told to "install yt-dlp" actually ends up.
    Packages(&'static str),
    /// Python's, which is where the matcher lives — it is not in `winget` at
    /// all, measured rather than assumed.
    Python(&'static str),
}

/// What this needs, and where each one comes from.
const TOOLS: [(&str, Source); 3] = [
    (DOWNLOADER, Source::Packages("yt-dlp.yt-dlp")),
    (CONVERTER, Source::Packages("Gyan.FFmpeg")),
    (MATCHER, Source::Python("spotdl")),
];

impl Source {
    /// What somebody would type to install it themselves.
    fn command(self) -> String {
        match self {
            Self::Packages(id) => format!("{PACKAGES} install {id}"),
            Self::Python(package) => format!("{PYTHON_PACKAGES} install {package}"),
        }
    }
}

/// Which programs a link needs.
///
/// Everything needs the downloader and the converter — the matcher hands its
/// work to both. A Spotify link needs the matcher as well, and nothing else
/// does: a listener who only ever pastes YouTube links must never be told to
/// install it.
fn needed_for(link: &str) -> Vec<(&'static str, Source)> {
    let matching = matches!(handler_for(link), LinkHandler::Matcher);

    TOOLS
        .into_iter()
        .filter(|(program, _)| matching || *program != MATCHER)
        .collect()
}

/// Starts `yt-dlp` and waits for it.
pub struct ExternalFetcher {
    /// Where each program's record of what it has already fetched is kept.
    ///
    /// Both of them keep one, in their own formats, and both skip what is in
    /// it: that is what turns a second press into a resume rather than a
    /// repeat. A playlist stopped at track twelve carries on at twelve, and a
    /// link pasted twice brings nothing the second time.
    ///
    /// The matcher was given none until a listener fetched the same playlist
    /// twice and got twenty second copies of tracks they already had
    /// (`MASTER_ISSUES` 104).
    ///
    /// Optional because a test fetching one link wants no memory of it.
    remembers: Option<PathBuf>,
}

/// What the downloader has fetched, in its format.
const DOWNLOADER_MEMORY: &str = "fetched.txt";

impl ExternalFetcher {
    /// Remembers what it has fetched, in `remembers`, which is a directory.
    #[must_use]
    pub const fn new(remembers: Option<PathBuf>) -> Self {
        Self { remembers }
    }
}

impl Default for ExternalFetcher {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ExternalFetcher {
    /// Installs one package through the machine's own package manager.
    fn install_with_packages(&self, id: &str) -> Result<()> {
        let packages = locate(PACKAGES).ok_or_else(|| {
            CoreError::invalid(
                "link",
                format!("{PACKAGES} is not on this machine, so nothing here can install anything"),
            )
        })?;

        // Every question answered in advance, because there is nobody to answer
        // them: this runs with no console and no input.
        let spoke = quietly(&packages)
            .args(["install", "--id", id, "--exact"])
            .args([
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output()
            .map_err(|err| CoreError::FileSystem(format!("{PACKAGES} would not start: {err}")))?;

        finished(&spoke, PACKAGES)
    }

    /// Installs or upgrades one package through Python's.
    ///
    /// `spotdl` is not in `winget` — searched rather than assumed — so this is
    /// the only way it arrives. A machine with no Python has no `pip` either,
    /// and the answer to that is a package manager away rather than something
    /// this can do quietly.
    fn install_with_python(&self, package: &str, upgrade: bool) -> Result<()> {
        let pip = locate(PYTHON_PACKAGES).ok_or_else(|| {
            CoreError::invalid(
                "link",
                format!(
                    "{PYTHON_PACKAGES} is not on this machine — install Python first:                      {PACKAGES} install Python.Python.3.12"
                ),
            )
        })?;

        let mut command = quietly(&pip);
        command.args(["install", "--disable-pip-version-check"]);
        if upgrade {
            command.arg("--upgrade");
        }

        let spoke = command
            .arg(package)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output()
            .map_err(|err| {
                CoreError::FileSystem(format!("{PYTHON_PACKAGES} would not start: {err}"))
            })?;

        finished(&spoke, PYTHON_PACKAGES)
    }
}

impl ExternalFetcher {
    /// What the matcher can say about a link without fetching anything.
    ///
    /// One metadata call, a couple of seconds, and it buys three things: what
    /// the list is called, how many tracks are in it, and therefore a
    /// percentage that means something. The answer is written to a file rather
    /// than printed, so it is read rather than parsed — `list_name` and
    /// `list_length` are fields, not sentences (`MASTER_ISSUES` 101).
    ///
    /// A link naming one track has no `list_name` at all, which is how a track
    /// and a list are told apart without guessing.
    ///
    /// Not knowing is not a failure: the download goes ahead, with no total and
    /// no playlist made from it.
    fn ask_matcher(
        &self,
        matcher: &Path,
        link: &str,
        workspace: &Path,
    ) -> (Option<String>, Vec<ListedTrack>) {
        let file = workspace.join("list.spotdl");

        let asked = quietly(matcher)
            .env("PYTHONIOENCODING", "utf-8")
            .arg("save")
            .arg(link)
            .arg("--save-file")
            .arg(&file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output();

        if !asked.is_ok_and(|spoke| spoke.status.success()) {
            return (None, Vec::new());
        }

        let Ok(text) = std::fs::read_to_string(&file) else {
            return (None, Vec::new());
        };
        let Ok(songs) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
            return (None, Vec::new());
        };

        let Some(first) = songs.first() else {
            return (None, Vec::new());
        };

        let list = first
            .get("list_name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);

        // Read as fields, not parsed out of anything it printed. `name` and
        // `artist` are what it writes into the tags, which is what the library
        // reads back — the two ends of the same string.
        let listed = songs
            .iter()
            .filter_map(|song| {
                Some(ListedTrack {
                    title: song.get("name")?.as_str()?.to_owned(),
                    artist: song.get("artist")?.as_str()?.to_owned(),
                })
            })
            .collect();

        (list, listed)
    }

    /// Fetches what a Spotify link *names*, which is not what it holds.
    ///
    /// **The matcher no longer downloads anything.** It is asked what the link
    /// names and where each of those recordings can be had — which is what it
    /// is actually good at — and the downloader fetches them, because that is
    /// what *it* is good at and because Cadenza already drives it properly:
    /// its own memory of what it has fetched, its own progress, and a failure
    /// that costs one track rather than the rest of the list.
    ///
    /// The measurements that led here, in order: the matcher brings about
    /// twenty tracks per run whatever the pace (`MASTER_ISSUES` 106), it dies
    /// outright on a track whose Spotify metadata is incomplete, and it will
    /// hand over the YouTube addresses if simply asked — fifty-two of them,
    /// bare, one per line (`MASTER_ISSUES` 107).
    ///
    /// What is given up is the tags: these files carry whatever the video
    /// carried. What is kept is the name, which is the half a listener reads —
    /// the file is called what the *record* is called, from the metadata, and
    /// a file with no title tag is titled by its name.
    fn fetch_matched(
        &self,
        link: &str,
        into: &Path,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> Result<FetchedTracks> {
        let matcher = locate(MATCHER)
            .ok_or_else(|| CoreError::invalid("link", format!("{MATCHER} is not installed")))?;
        let downloader = locate(DOWNLOADER)
            .ok_or_else(|| CoreError::invalid("link", format!("{DOWNLOADER} is not installed")))?;
        let converter = locate(CONVERTER)
            .ok_or_else(|| CoreError::invalid("link", format!("{CONVERTER} is not installed")))?;

        let workspace = workspace()?;
        let (list, listed) = self.ask_matcher(&matcher, link, &workspace);
        let addresses = self.ask_addresses(&matcher, link);

        // Both lists are the playlist's own order, so the address at a place
        // belongs to the track at that place. Where the two disagree in length
        // — a track the matcher could not place at all — the shorter one wins
        // and the extra is skipped rather than named after somebody else.
        let together: Vec<_> = addresses.iter().zip(listed.iter()).collect();
        if together.is_empty() {
            let _ = std::fs::remove_dir_all(&workspace);
            return Err(CoreError::invalid(
                "link",
                format!("{MATCHER} found nothing for that link"),
            ));
        }

        let wanted = u32::try_from(together.len()).unwrap_or(u32::MAX);
        for (done, (address, track)) in together.into_iter().enumerate() {
            if stop() {
                break;
            }

            let done = u32::try_from(done).unwrap_or(u32::MAX);
            progress(FetchProgress {
                percent: u8::try_from(done.saturating_mul(100) / wanted).unwrap_or(100),
                item: Some((done + 1, wanted)),
            });

            // One at a time, and a failure is one track. The matcher's own way
            // of doing this loses everything after the first refusal; the
            // downloader is asked once per recording, so a track that cannot
            // be had costs itself and nothing else.
            let _ = self.fetch_one(&downloader, &converter, address, track, &workspace);
        }

        let landed = land(&workspace, into)?;
        let _ = std::fs::remove_dir_all(&workspace);

        if landed.is_empty() {
            return Err(CoreError::invalid(
                "link",
                "nothing on that list could be fetched",
            ));
        }

        progress(FetchProgress {
            percent: 100,
            item: None,
        });
        Ok(FetchedTracks {
            files: landed,
            listed,
            playlist: list,
        })
    }

    /// Where each recording on the list can be had, in the list's own order.
    ///
    /// One search per track, and it is not quick — about sixteen seconds each,
    /// because it is a real search rather than a lookup. It is also the whole
    /// of what the matcher is for.
    ///
    /// Bare addresses, one to a line, mixed in with a few lines about what it
    /// is doing; anything that is not an address is not one of ours.
    fn ask_addresses(&self, matcher: &Path, link: &str) -> Vec<String> {
        let asked = quietly(matcher)
            .env("PYTHONIOENCODING", "utf-8")
            .arg("url")
            // The link before the options, and that is not a matter of taste:
            // `--audio` takes a *list*, so a link after it is swallowed as
            // another value and the program is left with nothing to look up.
            // It answers that with its usage message and no addresses at all,
            // which arrives here as "found nothing for that link"
            // (`MASTER_ISSUES` 108).
            .arg(link)
            // Two places to look, not its own one: twenty-two of the searches
            // on the list that reported this came back empty from the first
            // and were caught by the second (`MASTER_ISSUES` 103).
            .arg("--audio")
            .args(LOOK_IN)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output();

        let Ok(spoke) = asked else {
            return Vec::new();
        };

        String::from_utf8_lossy(&spoke.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.starts_with("https://")
                    && (line.contains("youtube.com/watch") || line.contains("youtu.be/"))
            })
            .map(str::to_owned)
            .collect()
    }

    /// Brings one recording down and names it after the record.
    ///
    /// No `--embed-metadata` here, and that is deliberate: what the video calls
    /// itself would land in the title tag and the library would show it. With
    /// no title tag a track is titled by its filename, and the filename is the
    /// one the list gave us.
    fn fetch_one(
        &self,
        downloader: &Path,
        converter: &Path,
        address: &str,
        track: &ListedTrack,
        workspace: &Path,
    ) -> Result<()> {
        let name = safe_name(&format!("{} - {}", track.artist, track.title));

        let spoke = quietly(downloader)
            .env("PYTHONIOENCODING", "utf-8")
            .args(["--extract-audio", "--audio-format", "mp3"])
            .args(["--audio-quality", "0"])
            .arg("--no-playlist")
            .arg("--embed-thumbnail")
            .arg("--ffmpeg-location")
            .arg(converter)
            .args(match self.remembers.as_ref() {
                Some(directory) => vec![
                    std::ffi::OsStr::new("--download-archive").to_owned(),
                    directory.join(DOWNLOADER_MEMORY).into_os_string(),
                ],
                None => Vec::new(),
            })
            .arg("--output")
            .arg(workspace.join(format!("{name}.%(ext)s")))
            .arg(address)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output()
            .map_err(|err| CoreError::FileSystem(format!("{DOWNLOADER} would not start: {err}")))?;

        finished(&spoke, DOWNLOADER)
    }
}

/// A name Windows will accept, from a name a record label chose.
///
/// The characters a path may not hold become spaces rather than disappearing:
/// `AC/DC` reads better as `AC DC` than as `ACDC`. Trailing dots and spaces go
/// too — Windows accepts them in an argument and then cannot open the file it
/// made, which is a very quiet way to lose a track.
fn safe_name(wanted: &str) -> String {
    let cleaned: String = wanted
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            other if (other as u32) < 0x20 => ' ',
            other => other,
        })
        .collect();

    // Capped for the same reason the other route caps it: everything before it
    // is a folder somebody chose, and the whole path has to stay inside what
    // Windows will accept (`MASTER_ISSUES` 90).
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    let mut short = String::new();
    for character in trimmed.chars() {
        if short.len() + character.len_utf8() > 150 {
            break;
        }
        short.push(character);
    }

    if short.is_empty() {
        "track".to_owned()
    } else {
        short
    }
}

/// A directory of our own to work in, named as short as it can be.
///
/// Everything after it is the track's own name, and the whole path has to stay
/// inside what Windows will accept — so every character spent naming this is a
/// character taken off the name of a track.
fn workspace() -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "cdz-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()) as u64
            ^ u64::from(std::process::id())
    ));
    std::fs::create_dir_all(&path)
        .map_err(|err| CoreError::FileSystem(format!("nowhere to download to: {err}")))?;
    Ok(path)
}

/// Moves every finished track out of the workspace and into the listener's
/// folder, in the order the names put them.
fn land(workspace: &Path, into: &Path) -> Result<Vec<PathBuf>> {
    let mut arrived = std::fs::read_dir(workspace)
        .map_err(|err| CoreError::FileSystem(format!("the download vanished: {err}")))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
        })
        .collect::<Vec<_>>();
    arrived.sort();

    let mut landed = Vec::with_capacity(arrived.len());
    for file in arrived {
        let place = free_name(into, &file);
        move_file(&file, &place)?;
        landed.push(place);
    }
    Ok(landed)
}

/// Whether a child worked, and what it said if it did not.
///
/// Read as bytes and decoded loosely, like everything else a child says here:
/// these programs draw progress bars and speak the machine's own language, and
/// neither is promised to be UTF-8 (`MASTER_ISSUES` 90).
fn finished(spoke: &std::process::Output, program: &str) -> Result<()> {
    if spoke.status.success() {
        return Ok(());
    }

    let mut said = String::from_utf8_lossy(&spoke.stdout).into_owned();
    said.push('\n');
    said.push_str(&String::from_utf8_lossy(&spoke.stderr));

    let last = said
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("it did not say why")
        .to_owned();

    Err(CoreError::invalid("link", format!("{program}: {last}")))
}

/// Where a program is, looking where Windows itself would.
///
/// `PATH` first, because that is where an installed program lives and how the
/// listener would run it themselves. Then beside Cadenza's own executable, for
/// somebody who dropped `yt-dlp.exe` next to the player rather than installing
/// anything — which is how most people actually have it.
fn locate(program: &str) -> Option<PathBuf> {
    let file = format!("{program}.exe");

    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(&file))
        .find(|candidate| candidate.is_file());

    on_path.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|beside| beside.join(&file)))
            .filter(|candidate| candidate.is_file())
    })
}

/// A command that runs without a console window of its own.
///
/// Cadenza is built for the windows subsystem so that it is a window and
/// nothing else. A child process started the ordinary way brings that console
/// back — one black rectangle per download, flashing over whatever the listener
/// was looking at. `CREATE_NO_WINDOW` is the flag that says a console is not
/// wanted; it is a plain constant, and this crate needs no `unsafe` to pass it.
fn quietly(program: &Path) -> Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut command = Command::new(program);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

/// Every line a child wrote, whatever bytes it chose to write them in.
///
/// Not `BufReader::lines()`, and this is the whole of `MASTER_ISSUES` 90.
/// `lines()` yields an error the moment it meets a byte sequence that is not
/// UTF-8, and the idiom for draining it — `map_while(Result::ok)` — treats that
/// error as the end. The reader stops, the pipe is dropped, and the program on
/// the other side is killed by its next `print`: *unable to open for writing:
/// [Errno 22] Invalid argument*. It looked like a download that failed and it
/// was a download we hung up on.
///
/// So: bytes, split on newlines, and whatever will not decode becomes U+FFFD.
/// A character we cannot read is a character drawn wrong in a progress line.
/// It is not a reason to abandon a track.
fn lines_of(stream: impl std::io::Read) -> impl Iterator<Item = String> {
    BufReader::new(stream)
        .split(b'\n')
        .map_while(std::result::Result::ok)
        .map(|line| {
            String::from_utf8_lossy(&line)
                .trim_end_matches('\r')
                .to_owned()
        })
}

/// The playlist's own name, out of `[download] Downloading playlist: Mixtape 11`.
///
/// Printed once, before the first track. It is the name the listener chose the
/// thing by, which is why it is worth reading a line for.
fn playlist_name(line: &str) -> Option<String> {
    let name = line
        .strip_prefix("[download] Downloading playlist: ")?
        .trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Which track of how many, out of a `[download] Downloading item 3 of 40`.
///
/// The only place yt-dlp says how long a playlist is. It says it again before
/// every item, which is what makes it a progress report rather than a header.
fn item_of(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("[download] Downloading item ")?;
    let (index, total) = rest.split_once(" of ")?;
    Some((
        index.trim().parse().ok()?,
        total.split_whitespace().next()?.parse().ok()?,
    ))
}

/// The percentage out of a `[download]  12.3% of  4.56MiB` line.
///
/// Read rather than parsed: the format is one program's console output and not
/// a protocol, so this takes what it recognises and ignores everything else.
/// A progress bar that misses a line costs nothing; one that fails to parse and
/// stops the download costs the track.
fn percentage(line: &str) -> Option<u8> {
    let (before, _) = line.split_once('%')?;
    let digits: String = before
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();

    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse::<f32>()
        .ok()
        .map(|percent| percent.clamp(0.0, 100.0) as u8)
}

/// A name in `directory` that is `wanted` and is not already taken.
///
/// Never overwrites. Somebody who fetches the same track twice gets a second
/// copy with a number on it, which is a thing they can delete; the alternative
/// is a file of theirs quietly replaced, which is not.
fn free_name(directory: &Path, wanted: &Path) -> PathBuf {
    let stem = wanted.file_stem().unwrap_or_default().to_string_lossy();
    let extension = wanted.extension().unwrap_or_default().to_string_lossy();

    let first = directory.join(wanted.file_name().unwrap_or_default());
    if !first.exists() {
        return first;
    }

    // Two is where a second copy starts, and the loop is bounded because a
    // hundred copies of one track is not a case worth carrying code for.
    (2..100)
        .map(|number| directory.join(format!("{stem} ({number}).{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

/// Moves a file, across drives if it has to.
///
/// `rename` is one operation and cannot half-happen, which is why it is tried
/// first. It also refuses to cross a volume, and the temporary directory is on
/// whichever disk Windows keeps its temporary files on — routinely not the one
/// holding somebody's music.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }

    std::fs::copy(from, to).map_err(|err| {
        CoreError::FileSystem(format!("the track could not be put in the library: {err}"))
    })?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// What the downloader said, plus what to do about it where that is knowable.
///
/// One case, and it earns itself. A `403` is what a site returns when the
/// downloader asked correctly and was refused anyway — which in practice means
/// its build is old enough that the site no longer accepts the client it
/// presents itself as. Passed through untouched it reads as a fault in Cadenza,
/// and two people hit it on two machines without either knowing there was
/// anything to do.
///
/// The remedy is `yt-dlp`'s own updater rather than whatever installed it: the
/// winget package sat two months behind the releases it packages, so the
/// obvious command reports that everything is current while nothing works.
fn explain(reason: &str) -> String {
    if reason.contains("403") || reason.contains("Forbidden") {
        return format!("{reason} — yt-dlp is likely out of date; run: yt-dlp -U");
    }

    reason.to_owned()
}

impl FetchPort for ExternalFetcher {
    fn missing_for(&self, link: &str) -> Vec<MissingTool> {
        needed_for(link)
            .into_iter()
            .filter(|(program, _)| locate(program).is_none())
            .map(|(program, source)| MissingTool {
                name: program.to_owned(),
                install: source.command(),
            })
            .collect()
    }

    fn install(&self, link: &str, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>> {
        for (program, source) in needed_for(link) {
            if locate(program).is_some() {
                continue;
            }

            said(&format!("installing {program}…"));
            let spoke = match source {
                Source::Packages(id) => self.install_with_packages(id),
                Source::Python(package) => self.install_with_python(package, false),
            };

            if let Err(err) = spoke {
                said(&format!("{program} was not installed: {err}"));
            }
        }

        // Asked again rather than inferred from the exit codes: what matters is
        // whether the program is there now, and that is a question with a
        // definite answer.
        Ok(self.missing_for(link))
    }

    fn update(&self, said: &dyn Fn(&str)) -> Result<String> {
        let Some(downloader) = locate(DOWNLOADER) else {
            return Err(CoreError::invalid(
                "link",
                format!("{DOWNLOADER} is not installed, so there is nothing to update"),
            ));
        };

        said(&format!("updating {DOWNLOADER}…"));

        let output = quietly(&downloader)
            .arg("--update")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .output()
            .map_err(|err| CoreError::FileSystem(format!("{DOWNLOADER} would not start: {err}")))?;

        // Its own words, whichever stream it chose to say them on: "up to date"
        // and "updated to stable@…" both arrive on stdout, and a refusal on
        // stderr.
        let mut spoke = String::from_utf8_lossy(&output.stdout).into_owned();
        spoke.push('\n');
        spoke.push_str(&String::from_utf8_lossy(&output.stderr));

        let mut last = spoke
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("yt-dlp said nothing")
            .to_owned();

        // And the matcher, where it is installed. It has no updater of its
        // own — it is a Python package, and the thing that installed it is the
        // thing that updates it.
        if locate(MATCHER).is_some() {
            said(&format!("updating {MATCHER}…"));
            match self.install_with_python("spotdl", true) {
                Ok(()) => last = format!("{last}; {MATCHER} up to date"),
                Err(err) => last = format!("{last}; {MATCHER} was not updated: {err}"),
            }
        }

        Ok(last)
    }

    fn fetch(
        &self,
        link: &str,
        into: &Path,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> Result<FetchedTracks> {
        // A link that names a recording without holding one goes to the
        // program that finds it. `what` does not apply there: a Spotify
        // address is a track or an album or a playlist by its own shape, and
        // both buttons fetch what it names.
        if matches!(handler_for(link), LinkHandler::Matcher) {
            return self.fetch_matched(link, into, progress, stop);
        }

        let downloader = locate(DOWNLOADER)
            .ok_or_else(|| CoreError::invalid("link", format!("{DOWNLOADER} is not installed")))?;
        let converter = locate(CONVERTER)
            .ok_or_else(|| CoreError::invalid("link", format!("{CONVERTER} is not installed")))?;

        // Downloaded somewhere nothing is watching, and moved in when it is
        // finished. `yt-dlp` writes a `.webm` or an `.m4a` first and converts
        // afterwards; landing those in a watched folder would have the library
        // importing an intermediate file that is about to be deleted, and the
        // listener watching a track appear and vanish.
        // Short on purpose. Everything after this directory is the video's
        // own title, and the whole path has to stay inside what Windows will
        // accept — so every character spent naming the workspace is a
        // character taken off the name of the track.
        let workspace = std::env::temp_dir().join(format!(
            "cdz-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos()) as u64
                ^ u64::from(std::process::id())
        ));
        std::fs::create_dir_all(&workspace)
            .map_err(|err| CoreError::FileSystem(format!("nowhere to download to: {err}")))?;

        let mut child = quietly(&downloader)
            // Said in UTF-8, so that the lines above arrive as the words they
            // are. Without it yt-dlp writes in the machine's own code page —
            // cp1252 here, cp1251 on a Russian one — and a track whose title
            // carries an en dash comes back with a hole in it. It is the
            // downloader's own knob: it is a Python program, and this is the
            // variable Python reads before it opens a stream.
            .env("PYTHONIOENCODING", "utf-8")
            .args(["--extract-audio", "--audio-format", "mp3"])
            .args(["--audio-quality", "0"])
            // One link is one track unless the listener said otherwise by
            // pressing the other button. A pasted address often carries a
            // playlist on the end of it, and nobody who pastes one link is
            // asking for two hundred without meaning to.
            //
            // `--ignore-errors` only with the playlist: one video that has
            // been taken down must not end the other thirty-nine, and what
            // landed is counted afterwards either way. For a single track
            // there is nothing to carry on with, and an error is the answer.
            .args(match what {
                FetchWhat::OneTrack => ["--no-playlist"].as_slice(),
                FetchWhat::WholePlaylist => ["--yes-playlist", "--ignore-errors"].as_slice(),
            })
            // Named for the converter we found, so that a machine with it in a
            // folder of its own rather than on PATH still works.
            .arg("--ffmpeg-location")
            .arg(&converter)
            .args(["--embed-metadata", "--embed-thumbnail"])
            // What has already been brought down, so that it is not brought
            // down twice. yt-dlp writes a line per finished track and reads
            // the same file before starting one.
            .args(match self.remembers.as_ref() {
                Some(directory) => vec![
                    std::ffi::OsStr::new("--download-archive").to_owned(),
                    directory.join(DOWNLOADER_MEMORY).into_os_string(),
                ],
                None => Vec::new(),
            })
            // A line per progress report rather than a redrawn bar, which is
            // the difference between something readable and a stream of
            // carriage returns.
            .arg("--newline")
            // The title, capped at 150 bytes.
            //
            // Not the fix for the "Error 22" people reported — that was ours
            // and is in `lines_of` (`MASTER_ISSUES` 90). This is for the other
            // half of it: a genuinely long title makes a path past the 260
            // characters Windows will accept, and yt-dlp fails with the same
            // EINVAL it fails with for everything (yt-dlp #11251, whose
            // reporter had a title of 214 characters).
            //
            // Bytes rather than characters, because that is what the limit is
            // made of: 150 bytes is 150 letters of Latin and about 75 of
            // Cyrillic, and both leave room for the folder in front of them.
            //
            // A playlist is numbered as well as named. Two videos in one
            // playlist can share a title, and the second would otherwise be
            // taken for the first already downloaded and skipped — and the
            // number is what puts an album back in its own order in any file
            // manager. It costs nothing in the library: the title there comes
            // from the tags `--embed-metadata` wrote, not from the filename.
            .arg("--output")
            .arg(workspace.join(match what {
                FetchWhat::OneTrack => "%(title).150B.%(ext)s",
                FetchWhat::WholePlaylist => "%(playlist_index)03d %(title).140B.%(ext)s",
            }))
            // Last, and after everything that could be read as an option. What
            // arrives here has already been checked for whitespace and for a
            // scheme (`link_policy`), so it cannot become a second argument.
            .arg(link)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|err| CoreError::FileSystem(format!("{DOWNLOADER} would not start: {err}")))?;

        // Drained on a thread rather than read at the end. A pipe nobody is
        // reading fills up, and a program writing into a full pipe stops — so
        // leaving this until after the download is how the download never
        // finishes.
        let complaints = child
            .stderr
            .take()
            .map(|stderr| std::thread::spawn(move || lines_of(stderr).collect::<Vec<_>>()));

        // Set the moment the listener asks to stop, so that what follows can
        // tell "they pressed stop" from "it failed" — the two look identical
        // from here: a killed program exits without success and says nothing.
        let mut stopped = false;

        let mut named = None;

        if let Some(stdout) = child.stdout.take() {
            let mut item = None;

            for line in lines_of(stdout) {
                if let Some(name) = playlist_name(&line) {
                    named = Some(name);
                }

                if stop() {
                    // Killed rather than asked. There is no polite way to end
                    // a download, and the workspace it was writing into is
                    // thrown away below along with whatever it left half
                    // written.
                    let _ = child.kill();
                    stopped = true;
                    break;
                }

                if let Some(counted) = item_of(&line) {
                    item = Some(counted);
                    progress(FetchProgress { percent: 0, item });
                } else if line.starts_with("[download]")
                    && let Some(percent) = percentage(&line)
                {
                    progress(FetchProgress { percent, item });
                } else if line.starts_with("[ExtractAudio]") {
                    // The download is done and the conversion has started.
                    // Not a percentage anybody reports, so it is a step rather
                    // than a number: near the end, and honestly not at it.
                    progress(FetchProgress { percent: 95, item });
                }
            }
        }

        let status = child
            .wait()
            .map_err(|err| CoreError::FileSystem(format!("{DOWNLOADER} did not finish: {err}")))?;

        let said = complaints
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();

        // What is there, before what the exit code says about it. A playlist
        // of forty with one video taken down comes back unsuccessful and with
        // thirty-nine tracks in it, and thirty-nine tracks is not a failure.
        // Neither is a playlist somebody stopped at twelve: eleven finished
        // tracks are eleven tracks they asked for, and throwing them away is
        // what would make pressing the button again download them twice.
        let mut arrived = std::fs::read_dir(&workspace)
            .map_err(|err| CoreError::FileSystem(format!("the download vanished: {err}")))?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
            })
            .collect::<Vec<_>>();
        // In the order the playlist was in, which is the order the filenames
        // were numbered in. `read_dir` promises nothing about its own.
        arrived.sort();

        if arrived.is_empty() {
            let _ = std::fs::remove_dir_all(&workspace);

            // Stopped before anything finished, or asked for what is already
            // here. Neither is a failure, and the caller says which by what it
            // asked for.
            if stopped || status.success() {
                return Ok(FetchedTracks::default());
            }

            // The last thing it said, not everything: yt-dlp explains itself in
            // one line and then prints where in its own source that happened.
            let reason = said
                .iter()
                .rev()
                .find(|line| line.contains("ERROR"))
                .map_or_else(
                    || "the download did not finish".to_owned(),
                    |line| line.trim_start_matches("ERROR: ").to_owned(),
                );
            return Err(CoreError::invalid("link", explain(&reason)));
        }

        let mut landed = Vec::with_capacity(arrived.len());
        for file in arrived {
            let place = free_name(into, &file);
            move_file(&file, &place)?;
            landed.push(place);
        }
        let _ = std::fs::remove_dir_all(&workspace);

        progress(FetchProgress {
            percent: 100,
            item: None,
        });
        Ok(FetchedTracks {
            files: landed,
            // The downloader names no list: what a YouTube playlist holds is
            // what it just fetched, and its own memory keeps that true.
            listed: Vec::new(),
            playlist: named,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{MATCHER, explain, free_name, needed_for, percentage};

    #[test]
    fn a_link_needs_only_what_it_needs() {
        // The promise this keeps: somebody who only ever pastes YouTube links
        // is never told to install the program that reads Spotify's names.
        let ordinary: Vec<&str> = needed_for("https://www.youtube.com/watch?v=abc")
            .into_iter()
            .map(|(program, _)| program)
            .collect();
        assert!(!ordinary.contains(&MATCHER));
        assert_eq!(ordinary.len(), 2, "the downloader and the converter");

        let named: Vec<&str> = needed_for("https://open.spotify.com/track/abc")
            .into_iter()
            .map(|(program, _)| program)
            .collect();
        assert!(named.contains(&MATCHER), "and this one needs all three");
        assert_eq!(named.len(), 3);
    }

    #[test]
    fn a_refusal_carries_the_one_thing_that_answers_it() {
        let refused = explain("unable to download video data: HTTP Error 403: Forbidden");
        assert!(refused.contains("403"), "what it said is still there");
        assert!(refused.contains("yt-dlp -U"), "and what to do about it");

        // Everything else is passed through as it came. A guess appended to an
        // error nobody understands is worse than the error alone.
        assert_eq!(
            explain("Video unavailable. This video is private"),
            "Video unavailable. This video is private"
        );
    }

    #[test]
    fn a_percentage_is_read_out_of_a_progress_line() {
        assert_eq!(percentage("[download]   3.4% of    4.56MiB"), Some(3));
        assert_eq!(percentage("[download] 100.0% of    4.56MiB"), Some(100));
        assert_eq!(percentage("[download]  12% of 4MiB"), Some(12));

        // Lines that are not progress, which is most of them.
        assert_eq!(percentage("[download] Destination: a.webm"), None);
        assert_eq!(percentage("[info] something"), None);
        assert_eq!(percentage(""), None);
    }

    #[test]
    fn a_name_already_taken_gets_a_number_rather_than_taking_the_file() {
        let root = std::env::temp_dir().join(format!("cadenza-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a directory");

        let wanted = std::path::Path::new("A Track.mp3");
        assert_eq!(free_name(&root, wanted), root.join("A Track.mp3"));

        std::fs::write(root.join("A Track.mp3"), b"first").expect("written");
        assert_eq!(free_name(&root, wanted), root.join("A Track (2).mp3"));

        std::fs::write(root.join("A Track (2).mp3"), b"second").expect("written");
        assert_eq!(free_name(&root, wanted), root.join("A Track (3).mp3"));

        // And the first one is still what it was, which is the whole point.
        assert_eq!(
            std::fs::read(root.join("A Track.mp3")).expect("still there"),
            b"first"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
