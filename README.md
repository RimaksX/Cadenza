# Cadenza

A local music player for Windows.

Your music is files on your disk, and nothing here depends on a service being
up. No account, no cloud, no telemetry, no neural networks, and no network
client of its own — close the connection and everything still works.

- Library built from folders you point it at, kept current while the window is
  open: files copied in are noticed, files removed are noticed
- MP3, AAC, ALAC, FLAC and WAV
- Playlists, a queue you arrange by hand, shuffle that listens to what fits, and
  a radio built from your own library
- Crossfade and gapless playback
- Three-knob and eight-band parametric equaliser
- A visualiser that follows what is playing
- What you listened to, kept for thirty days — or not kept at all, if you say so
- Several listeners on one machine, each with their own library, playlists,
  history and settings
- Paste a link and the track joins the library

## What you need

Windows 10 or 11, 64-bit.

## Building it

There is no installer yet. Until there is, build it yourself — you need
[Rust](https://rustup.rs) and nothing else:

```
cargo build --release
```

The player is then `target/release/cadenza.exe`. It is self-contained: the
fonts and the icons are inside it.

## Bringing a track in from a link

The field at the top of the library takes a web address and puts the track in
your Cadenza folder.

Cadenza does not download anything itself. It runs **yt-dlp**, which you
install once:

```
winget install yt-dlp.yt-dlp
```

That pulls in ffmpeg with it, which is what turns what arrives into an mp3.
Neither program is shipped with Cadenza, and that is deliberate: they are kept
current by the people who watch the sites they fetch from, so the feature does
not quietly stop working between releases here. Restart Cadenza after
installing them — it looks for them when it starts.

Some services cannot be fetched from at all. Spotify, Apple Music, Tidal and
Deezer hand out encrypted audio to their own players, so there is no recording
at those addresses for anything to find. Cadenza says so rather than trying.

## Where your things are

Everything Cadenza keeps lives in one place:

```
%LOCALAPPDATA%\Cadenza
```

The library database, the cover art it has cached, and its log. Downloaded
tracks go to a folder of their own — `Music\Cadenza` under your user folder —
and the music you already had is never moved, copied or written to. Cadenza
reads your folders and remembers where things are; it does not tidy them.

## Licence

GPL-3.0-only — version 3 of the GNU General Public License and no other
version. That is not a preference: the interface is built on
[Slint](https://slint.dev), which is used here under GPLv3, so this is the
version the whole work can be offered under. The full text is in
[LICENSE](LICENSE).
