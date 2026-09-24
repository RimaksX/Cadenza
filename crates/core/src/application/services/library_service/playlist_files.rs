//! Playlists as files: written out for another player, read in from one.

use std::collections::HashSet;

use super::*;
use crate::domain::ids::PlaylistId;
use crate::domain::policies::m3u_policy::{self, M3uEntry};
use crate::domain::ports::folder_picker::FileKind;

/// What reading a playlist file came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistImport {
    /// The playlist it went into: named after the file.
    pub name: String,
    /// How many of its tracks are in that playlist now.
    pub joined: usize,
    /// How many tracks the file listed.
    pub listed: usize,
}

impl LibraryService {
    /// Asks where, and writes a playlist out as an `.m3u8` another player can
    /// read. `None` when nothing was chosen.
    pub fn export_playlist(&self, playlist_id: PlaylistId) -> Result<Option<PathBuf>> {
        let playlists = self.playlists()?;
        let playlist = playlists.get(playlist_id)?;
        let suggested = format!("{}.m3u8", file_name(playlist.name.as_str()));
        let Some(to) =
            self.ports
                .picker
                .save_file("Export a playlist", FileKind::Playlist, &suggested)?
        else {
            return Ok(None);
        };

        let mut entries = Vec::new();
        for track in playlists.tracks_of(playlist_id)? {
            let Some(file) = self.ports.media_files.get(track.media_file_id)? else {
                continue;
            };
            let label = match &track.artist {
                Some(artist) => format!("{artist} - {}", track.title),
                None => track.title.clone(),
            };
            entries.push(M3uEntry {
                path: file.path,
                seconds: track.duration.as_millis() / 1_000,
                label,
            });
        }

        self.ports
            .saved
            .write(&to, m3u_policy::write(&entries, &to).as_bytes())?;
        Ok(Some(to))
    }

    /// Asks which file, and reads a playlist in from it: into a playlist named
    /// after the file, made if there is none and added to if there is, the
    /// way a playlist fetched twice is.
    ///
    /// A file it lists that is not in the library yet is brought in, the way
    /// a file dropped on the window is: choosing a list is choosing what is on
    /// it. A file that is not there any more is passed over, and counted.
    /// `None` when nothing was chosen.
    pub fn import_playlist(&self) -> Result<Option<PlaylistImport>> {
        let playlists = self.playlists()?;
        let Some(from) = self
            .ports
            .picker
            .pick_file("Import a playlist", FileKind::Playlist)?
        else {
            return Ok(None);
        };
        let profile_id = self.context.require_active_profile()?;
        let paths = m3u_policy::read(&self.ports.files.read(&from)?, &from);

        let name = from
            .file_stem()
            .map(|stem| stem.to_string_lossy().trim().to_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "Imported".to_owned());
        let existing = playlists
            .list()?
            .into_iter()
            .map(|summary| summary.playlist)
            .find(|playlist| playlist.name.as_str().eq_ignore_ascii_case(&name));
        let playlist = match existing {
            Some(playlist) => playlist,
            None => playlists.create(&name)?,
        };

        // What it holds already counts, so reading the same file twice adds
        // nothing twice.
        let mut held: HashSet<MediaFileId> = playlists
            .tracks_of(playlist.id)?
            .into_iter()
            .map(|track| track.media_file_id)
            .collect();
        let mut joined = 0;
        let mut brought = false;

        for path in &paths {
            let Ok(metadata) = self.ports.files.metadata(path) else {
                continue;
            };
            if metadata.is_dir || !has_supported_extension(path) {
                continue;
            }
            match self.import_file(profile_id, path, metadata.size, metadata.modified, true) {
                Ok(Imported::Added | Imported::Updated) => brought = true,
                Ok(Imported::Unchanged | Imported::Duplicate) => {}
                Err(err) => {
                    self.record_failure(profile_id, path, &err)?;
                    continue;
                }
            }
            let Some(file) = self.ports.media_files.find_by_path(path)? else {
                continue;
            };
            if !held.insert(file.id) || playlists.add_track(playlist.id, file.id).is_ok() {
                joined += 1;
            } else {
                // Waiting for a decision about a duplicate: not in the
                // library, so not in a list either, until it is decided.
                held.remove(&file.id);
            }
        }

        if brought {
            self.context.events.publish(DomainEvent::LibraryChanged);
        }
        Ok(Some(PlaylistImport {
            name: playlist.name.as_str().to_owned(),
            joined,
            listed: paths.len(),
        }))
    }

    fn playlists(&self) -> Result<&Arc<super::super::PlaylistService>> {
        self.ports
            .playlists
            .as_ref()
            .ok_or_else(|| CoreError::invalid("playlist", "this copy keeps no playlists"))
    }
}

/// A playlist's name as a file's: without the characters Windows refuses in
/// one.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if r#"<>:"/\|?*"#.contains(character) || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_owned();
    if cleaned.is_empty() {
        "Playlist".to_owned()
    } else {
        cleaned
    }
}
