//! Playlist files: what an `.m3u8` written here says, and what one written
//! anywhere else can be read as.
//!
//! The format has no standard beyond what players agree on: a list of paths,
//! one to a line, with `#` lines for everything else. `#EXTM3U` and `#EXTINF`
//! are the two every player reads, so they are the two this writes.

use std::path::{Path, PathBuf};

/// One track as a playlist file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M3uEntry {
    pub path: PathBuf,
    /// Whole seconds, as `#EXTINF` has it.
    pub seconds: u64,
    /// `Artist - Title`, or the title alone.
    pub label: String,
}

/// The text of a playlist file for `entries`, saved at `saved_as`.
///
/// A track inside the folder the file is saved in is written relative to it,
/// so the folder can be copied to another computer or a phone with its
/// playlist still true; anything else is written as the whole path, which is
/// the only thing that can find it.
#[must_use]
pub fn write(entries: &[M3uEntry], saved_as: &Path) -> String {
    let base = saved_as.parent();
    let mut text = String::from("#EXTM3U\n");
    for entry in entries {
        let path = base
            .and_then(|base| entry.path.strip_prefix(base).ok())
            .unwrap_or(&entry.path);
        // A line break in a tag would start a line that is not a path.
        let label = entry.label.replace(['\r', '\n'], " ");
        text.push_str(&format!(
            "#EXTINF:{},{label}\n{}\n",
            entry.seconds,
            path.display()
        ));
    }
    text
}

/// The files a playlist file lists, in its order, read from its bytes.
///
/// UTF-8 when it is, and the Windows Cyrillic code page when it is not: an
/// `.m3u` from an older player on a Russian system is in that, and read as
/// UTF-8 every name in it would be noise. Relative paths are relative to the
/// file; addresses on the web are not files and are left out.
#[must_use]
pub fn read(bytes: &[u8], saved_at: &Path) -> Vec<PathBuf> {
    let text = decode(bytes);
    let base = saved_at.parent().unwrap_or_else(|| Path::new(""));
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = match line.strip_prefix("file:///") {
                Some(rest) => percent_decode(rest),
                None if line.contains("://") => return None,
                None => line.to_owned(),
            };
            let path = PathBuf::from(line);
            Some(if path.is_absolute() {
                path
            } else {
                base.join(path)
            })
        })
        .collect()
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| windows_1251(byte)).collect(),
    }
}

/// A byte of Windows-1251: the alphabet, and `Ё`/`ё`, which are what names
/// are written in. The punctuation above 0x7F is rare enough in a file name
/// to be left as a mark rather than tabled.
fn windows_1251(byte: u8) -> char {
    match byte {
        0x00..=0x7F => char::from(byte),
        0xA8 => 'Ё',
        0xB8 => 'ё',
        0xC0..=0xFF => char::from_u32(0x0410 + u32::from(byte - 0xC0)).unwrap_or('\u{FFFD}'),
        _ => '\u{FFFD}',
    }
}

/// `%20` and the like, as a `file://` address spells a space. A sequence that
/// is not valid UTF-8 once decoded is kept as it was written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_list_reads_back_as_the_same_files() {
        let saved = Path::new(r"C:\Music\lists\evening.m3u8");
        let entries = vec![
            M3uEntry {
                path: PathBuf::from(r"C:\Music\lists\near\one.mp3"),
                seconds: 187,
                label: "Artist - One".into(),
            },
            M3uEntry {
                path: PathBuf::from(r"D:\Elsewhere\two.flac"),
                seconds: 64,
                label: "Two\nbroken".into(),
            },
        ];
        let text = write(&entries, saved);
        assert!(text.starts_with("#EXTM3U\n#EXTINF:187,Artist - One\n"));
        // Inside the file's own folder: relative. Outside it: whole.
        assert!(text.contains("\nnear\\one.mp3\n"), "{text}");
        assert!(text.contains("\nD:\\Elsewhere\\two.flac\n"), "{text}");
        assert!(text.contains("#EXTINF:64,Two broken\n"), "{text}");

        let paths = read(text.as_bytes(), saved);
        assert_eq!(
            paths,
            vec![entries[0].path.clone(), entries[1].path.clone()]
        );
    }

    #[test]
    fn other_players_lists_are_read_as_they_write_them() {
        let saved = Path::new(r"C:\Lists\old.m3u");
        // A byte-order mark, a web address, a file address with a space, and
        // a relative path.
        let text = "\u{FEFF}#EXTM3U\r\nhttps://radio.example/stream\r\nfile:///C:/Music/My%20Song.mp3\r\n\r\nsub/three.ogg\r\n";
        assert_eq!(
            read(text.as_bytes(), saved),
            vec![
                PathBuf::from("C:/Music/My Song.mp3"),
                PathBuf::from(r"C:\Lists").join("sub/three.ogg"),
            ]
        );
    }

    #[test]
    fn a_list_from_an_old_russian_player_keeps_its_names() {
        // "C:\Музыка\Ёлка.mp3" in Windows-1251.
        let mut bytes = b"C:\\".to_vec();
        bytes.extend([0xCC, 0xF3, 0xE7, 0xFB, 0xEA, 0xE0]);
        bytes.push(b'\\');
        bytes.extend([0xA8, 0xEB, 0xEA, 0xE0]);
        bytes.extend(b".mp3");
        assert_eq!(
            read(&bytes, Path::new(r"C:\x.m3u")),
            vec![PathBuf::from(r"C:\Музыка\Ёлка.mp3")]
        );
    }
}
