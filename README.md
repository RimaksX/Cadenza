# Cadenza

A local music player for Windows.

Your music is files on your disk, and nothing here depends on a service being
up. No account, no cloud, no telemetry, no neural networks, and no network
client of its own - close the connection and everything still works.

- Library built from folders you point it at, kept current while the window is
  open: files copied in are noticed, files removed are noticed
- MP3, AAC, ALAC, FLAC and WAV
- Playlists, a queue you arrange by hand, shuffle that listens to what fits, and
  a radio built from your own library
- Crossfade and gapless playback
- Three-knob and eight-band parametric equaliser, kept per track
- What you listened to, kept for thirty days - or not kept at all, if you say so
- Several listeners on one machine, each with their own library, playlists,
  history and settings
- Paste a link and the track joins the library - one track, or a whole playlist

## What you need

Windows 10 or 11, 64-bit.

## Installing it

Take the `.msi` from
[Releases](https://github.com/RimaksX/Cadenza/releases) and run it. It installs
for the current user only and never asks for administrator, and a later version
replaces this one rather than standing beside it.

The installer is not signed, so Windows shows a SmartScreen warning before it
will run it: choose **More info**, then **Run anyway**.

## Building it yourself

You need [Rust](https://rustup.rs) and nothing else:

```
cargo build --release
```

The player is then `target/release/cadenza.exe`. It is self-contained: the
fonts and the icons are inside it.

## Bringing a track in from a link

The field at the top of the library takes a web address and puts what it finds
in your Cadenza folder. **GET** brings down one track. **PLAYLIST** brings down
a whole list and makes a playlist of it, under the name the source gave it.
While either runs, the button says which track of how many it is on, and
pressing it again stops it - what has already landed is kept.

Cadenza downloads nothing itself. It runs **yt-dlp** to fetch, **ffmpeg** to
turn what arrives into an mp3, and **spotDL** to read what a Spotify link names.

**It installs those itself.** The first link that needs a program it has not got
comes back with an offer and an install button instead of a download; a link
that fails because yt-dlp has fallen behind comes back with an offer to update
it. yt-dlp and ffmpeg come from winget, and so does Python, when a Spotify link
needs it on a machine that has never had one; spotDL comes from Python's own
pip, which is the only place it exists. Nothing is installed without that press,
and no restart is needed after one. They are not shipped inside Cadenza deliberately: they are
kept current by the people who watch the sites they fetch from, so the feature
does not quietly stop working between releases here.

**Spotify links work, and not in the way that phrase suggests.** Nothing takes
audio out of Spotify - its streams are encrypted, and every tool that claims
otherwise does what this does. The link is read for the titles, artists and
albums it names, and each of those recordings is then found and downloaded from
YouTube. The audio is the same audio the other button gets, with better names
on it.

Apple Music, Tidal and Deezer are refused rather than attempted. Their audio is
encrypted too, and nothing here can read names off one of their links the way it
can off a Spotify one, so there is nothing at those addresses to work from.

## Where your things are

The library database and the settings follow your Windows profile:

```
%APPDATA%\Cadenza
```

The cover art it has cached, its log, and its record of what it has already
fetched stay on the machine, because they are large and can be made again:

```
%LOCALAPPDATA%\Cadenza
```

Downloaded tracks go to a folder of their own - `Music\Cadenza` under your user
folder - and the music you already had is never moved, copied or written to.
Cadenza reads your folders and remembers where things are; it does not tidy
them.

## Licence

GPL-3.0-only - version 3 of the GNU General Public License and no other
version. That is not a preference: the interface is built on
[Slint](https://slint.dev), which is used here under GPLv3, so this is the
version the whole work can be offered under. The full text is in
[LICENSE](LICENSE).
