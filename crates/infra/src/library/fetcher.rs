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

use cadenza_core::domain::ports::fetcher::{
    FetchPort, FetchProgress, FetchWhat, FetchedTracks, MissingTool,
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

/// What this needs, and what each one is called where it is installed from.
///
/// The identifier is exact on purpose. `winget install yt-dlp` matches both the
/// package and something else in the Microsoft Store and refuses to choose,
/// which is where somebody told to "install yt-dlp" actually ends up.
const TOOLS: [(&str, &str); 2] = [(DOWNLOADER, "yt-dlp.yt-dlp"), (CONVERTER, "Gyan.FFmpeg")];

/// Starts `yt-dlp` and waits for it.
pub struct ExternalFetcher {
    /// Where the list of what has already been brought down is kept.
    ///
    /// yt-dlp appends one line per finished track and skips anything already
    /// in it, which is what turns a second press into a resume rather than a
    /// repeat: a playlist stopped at track twelve carries on at twelve, and a
    /// link pasted twice brings nothing the second time.
    ///
    /// Optional because a test fetching one link wants no memory of it.
    archive: Option<PathBuf>,
}

impl ExternalFetcher {
    /// Remembers what it has fetched in the file at `archive`.
    #[must_use]
    pub const fn new(archive: Option<PathBuf>) -> Self {
        Self { archive }
    }
}

impl Default for ExternalFetcher {
    fn default() -> Self {
        Self::new(None)
    }
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
    fn missing(&self) -> Vec<MissingTool> {
        TOOLS
            .iter()
            .filter(|(program, _)| locate(program).is_none())
            .map(|(program, package)| MissingTool {
                name: (*program).to_owned(),
                install: format!("{PACKAGES} install {package}"),
            })
            .collect()
    }

    fn install(&self, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>> {
        let Some(packages) = locate(PACKAGES) else {
            return Err(CoreError::invalid(
                "link",
                format!(
                    "{PACKAGES} is not on this machine, so nothing here can install anything —                      the two programs can still be installed by hand"
                ),
            ));
        };

        for (program, package) in TOOLS {
            if locate(program).is_some() {
                continue;
            }

            said(&format!("installing {program}…"));

            let output = quietly(&packages)
                .args(["install", "--id", package, "--exact"])
                // Every question answered in advance, because there is nobody
                // to answer them: this runs with no console and no input.
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
                .map_err(|err| {
                    CoreError::FileSystem(format!("{PACKAGES} would not start: {err}"))
                })?;

            // Read as bytes and decoded loosely, like everything else a child
            // says here: `winget` draws progress bars and speaks the machine's
            // own language, and neither is promised to be UTF-8
            // (`MASTER_ISSUES` 90).
            let last = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .unwrap_or_default()
                .to_owned();

            if !output.status.success() {
                said(&format!("{program} was not installed: {last}"));
            }
        }

        // Asked again rather than inferred from the exit codes: what matters is
        // whether the program is there now, and that is a question with a
        // definite answer.
        Ok(self.missing())
    }

    fn update(&self, said: &dyn Fn(&str)) -> Result<String> {
        let Some(downloader) = locate(DOWNLOADER) else {
            return Err(CoreError::invalid(
                "link",
                format!("{DOWNLOADER} is not installed, so there is nothing to update"),
            ));
        };

        said("updating yt-dlp…");

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

        let last = spoke
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("yt-dlp said nothing")
            .to_owned();

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
            .args(match self.archive.as_ref() {
                Some(archive) => vec![
                    std::ffi::OsStr::new("--download-archive"),
                    archive.as_os_str(),
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
            playlist: named,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{explain, free_name, percentage};

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
