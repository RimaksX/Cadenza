//! Fetching from a link into the library, and the tools that do it.

use super::*;

impl LibraryService {
    /// Brings down whatever is at `link` and puts it in the library.
    ///
    /// Long: this runs a download and a conversion, so it belongs on a thread
    /// and never on the one drawing the window. `progress` is called with whole
    /// percentages as they arrive, on that same thread.
    ///
    /// The order of the three checks is the order the listener can act on them.
    /// Whether the link is a link is instant and theirs to fix; whether the
    /// tools exist is a five-minute install; whether there is a folder is one
    /// press. Discovering the third after waiting for a download would be a
    /// download thrown away.
    pub fn fetch_from_link(
        &self,
        link: &str,
        what: FetchWhat,
        progress: &dyn Fn(FetchProgress),
        stop: &dyn Fn() -> bool,
    ) -> Result<Fetched> {
        let profile_id = self.context.require_active_profile()?;
        let link = link.trim();

        if !is_a_link(link) {
            return Err(CoreError::invalid(
                "link",
                "that is not a web address — paste the whole thing, starting with https://",
            ));
        }

        // Before the tools, because this is true whatever is installed: no
        // version of anything will ever fetch from these, and the listener's
        // next move is the same track somewhere that will part with it.
        //
        // Spotify used to be on that list and is not any more, and the
        // distinction is worth keeping straight: nothing can take Spotify's
        // *audio*, which is still true, but its links name a recording and the
        // recording can be found. That is what the matcher does, and what every
        // service claiming to "download from Spotify" does.
        if let LinkHandler::Refused(service) = handler_for(link) {
            return Err(CoreError::invalid(
                "link",
                format!(
                    "{service} encrypts its audio — nothing can fetch it.                      Find the track on YouTube instead"
                ),
            ));
        }

        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        let missing = fetcher.missing_for(link);
        if !missing.is_empty() {
            return Ok(Fetched::NeedsTools(missing));
        }

        let Some(folder) = self.local_folder()? else {
            // Not an error: the listener was asked once whether Cadenza could
            // make itself a folder and said no, which was a fair answer to a
            // question about their disk. Now there is a reason, so the offer
            // comes back with one.
            let would_be = self.suggested_folder().ok_or_else(|| {
                CoreError::invalid("local folder", "this machine has no music folder")
            })?;
            return Ok(Fetched::NeedsLocalFolder(would_be));
        };

        // What this listener already has, by the same rule the playlist uses
        // to find them: the tags a track carries, compared with the names the
        // list gives. Read once rather than per track.
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;
        let have = |track: &ListedTrack| {
            library
                .iter()
                .any(|summary| summary_is(summary, &track.title, &track.artist))
        };

        let brought = match fetcher.fetch(link, &folder, what, progress, stop, &have) {
            Ok(brought) => brought,
            // A refusal that reads like a stale copy is an offer rather than an
            // error: the listener can fix it by pressing one thing, and being
            // told so beats being told what went wrong.
            Err(CoreError::Invalid { field, reason }) if looks_out_of_date(&reason) => {
                debug_assert_eq!(field, "link");
                return Ok(Fetched::NeedsUpdate(reason));
            }
            Err(err) => return Err(err),
        };
        let files = brought.files;

        // From here they are ordinary files that appeared in a watched folder,
        // and they go through the same import as one somebody copied in — the
        // same tags, the same duplicate check, the same review queue when one
        // cannot be read. A track is a track however it arrived.
        for file in &files {
            let metadata = self.ports.files.metadata(file)?;
            self.import_file(profile_id, file, metadata.size, metadata.modified, true)?;
        }

        // A playlist that came in as a playlist becomes one here, under the
        // name it had where it came from. Forty tracks landing loose in a
        // library is forty tracks somebody has to gather up by hand — and the
        // thing they were part of is exactly what they pasted.
        //
        // Failing to make it is not failing to fetch: the tracks are in the
        // library either way, and that is what was asked for.
        if let Some(name) = brought.playlist.as_deref()
            && let Err(err) = self.gather_into_playlist(name, &files, &brought.listed)
        {
            self.context.warn(&format!(
                "the tracks came in but the playlist did not: {err}"
            ));
        }

        // Nothing new on the disk, and that is not nothing done: the list above
        // was still rebuilt from what the listener already has, which is the
        // whole point of pressing it a second time.
        if files.is_empty() {
            return Ok(Fetched::NothingNew);
        }
        self.context.events.publish(DomainEvent::LibraryChanged);

        // One track is named; forty are counted. Naming the first of forty
        // would be a line that answers a question nobody asked.
        if let (FetchWhat::OneTrack, Some(file)) = (what, files.first()) {
            return Ok(Fetched::Landed(
                file.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }

        Ok(Fetched::LandedMany(files.len()))
    }

    /// Puts what just arrived into a playlist of that name, making it if it is
    /// new and adding to it if it is not.
    ///
    /// Adding rather than refusing, because that is what a second press means:
    /// a playlist fetched again brings whatever was added to it since, and
    /// those tracks belong with the ones already here. A track already in the
    /// playlist is not added twice — `add_track` is what decides that.
    pub(super) fn gather_into_playlist(
        &self,
        name: &str,
        files: &[PathBuf],
        listed: &[ListedTrack],
    ) -> Result<()> {
        let Some(playlists) = self.ports.playlists.as_ref() else {
            return Ok(());
        };

        let existing = playlists
            .list()?
            .into_iter()
            .map(|summary| summary.playlist)
            .find(|playlist| playlist.name.as_str().eq_ignore_ascii_case(name));

        let playlist = match existing {
            Some(playlist) => playlist,
            None => playlists.create(name)?,
        };

        // Everything the list names that this listener has, whether it came
        // just now or a week ago.
        //
        // A playlist holds the *list*. A second fetch of the same address
        // fetches almost nothing — the memory sees to that — so a playlist
        // built from what arrived would hold the two tracks that happened to be
        // new and none of the fifty that were already here.
        //
        // Matched on title and artist rather than on a file name: both ends of
        // that comparison came from the same metadata — the matcher wrote the
        // tags, the library read them back — while a file name has been through
        // one program's idea of what a filename may contain.
        let profile_id = self.context.require_active_profile()?;
        let library = self.ports.tracks.summaries_for_profile(profile_id)?;

        // What it already holds counts as added, or fetching the same list
        // twice would put every track in it twice — and `add_track` appends
        // whatever it is given, as it should: it is the caller who knows
        // whether this is the same list coming round again.
        let mut added: std::collections::HashSet<_> = playlists
            .tracks_of(playlist.id)?
            .into_iter()
            .map(|track| track.media_file_id)
            .collect();

        for track in listed {
            let found = library
                .iter()
                .find(|summary| summary_is(summary, &track.title, &track.artist));

            if let Some(summary) = found
                && added.insert(summary.media_file_id)
                && playlists
                    .add_track(playlist.id, summary.media_file_id)
                    .is_err()
            {
                added.remove(&summary.media_file_id);
            }
        }

        // One track that will not join must not cost the other fifty-one.
        //
        // It used to. A file the import set aside — a duplicate of one already
        // in the library, which is what a second fetch of the same list is full
        // of — is not in the library, `add_track` says so, and the `?` here
        // threw away every track after it. A listener watched three new tracks
        // arrive and the playlist stay exactly where it was.
        //
        // And whatever arrived that the list did not name, or named
        // differently: the tracks a YouTube link brought, and any whose tags
        // the listener has since corrected.
        let mut missed = 0;
        for file in files {
            let joined = match self.ports.media_files.find_by_path(file)? {
                Some(media_file) => {
                    !added.insert(media_file.id)
                        || playlists.add_track(playlist.id, media_file.id).is_ok()
                }
                None => false,
            };
            if !joined {
                missed += 1;
            }
        }

        if missed > 0 {
            self.context.warn(&format!(
                "{missed} of {} did not join the playlist: they are already in                  the library, or waiting for a decision",
                files.len()
            ));
        }

        Ok(())
    }

    /// Installs the programs a fetch needs, through the machine's own package
    /// manager, and says what is still missing afterwards.
    ///
    /// Empty means it worked. `said` is called as it goes, because installing
    /// something on somebody's computer is not a thing to do behind a spinner.
    pub fn install_tools(&self, link: &str, said: &dyn Fn(&str)) -> Result<Vec<MissingTool>> {
        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        fetcher.install(link, said)
    }

    /// Brings the downloader up to date, and hands back what it said about it.
    pub fn update_downloader(&self, said: &dyn Fn(&str)) -> Result<String> {
        let fetcher = self.ports.fetcher.as_ref().ok_or_else(|| {
            CoreError::invalid("link", "this copy cannot fetch anything from a link")
        })?;

        fetcher.update(said)
    }
}
