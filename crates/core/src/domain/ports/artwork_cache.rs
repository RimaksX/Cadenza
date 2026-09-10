//! Storing cover images outside the database.

use std::path::PathBuf;

use crate::Result;
use crate::domain::ids::{MediaFileId, PlaylistId, ProfileId};

/// Whose cover an image is.
///
/// Three shapes rather than three ports, because the storage is one thing —
/// bytes on disk under a key — and only the key differs. Where the key lives is
/// what says whether the image is a fact about a recording or a choice somebody
/// made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverOf {
    /// The recording's own, as its file carried it.
    ///
    /// A fact about the file and shared by everyone on the machine: the same
    /// bytes came out of the same tag.
    Track(MediaFileId),
    /// What one listener chose for that recording.
    ///
    /// A local override in the sense of PROJECT_MASTER 2.1, and per profile for
    /// the same reason a corrected title is (rule 12.1): choosing a cover for
    /// yourself must not choose it for whoever else uses this machine.
    ChosenTrack(ProfileId, MediaFileId),
    /// What a listener chose for one of their lists.
    ///
    /// No profile in the key because a playlist already belongs to one.
    Playlist(PlaylistId),
    /// The face beside a listener's name.
    ///
    /// A picture of a person rather than of a record, and the only key here
    /// that stands for somebody instead of something they own. It goes when
    /// they do: deleting a profile takes the file with it, because a portrait
    /// left behind is the one piece of a deleted listener that would still be
    /// on the disk (`MASTER_ISSUES` 148).
    Profile(ProfileId),
}

/// Cover art storage.
///
/// Artwork lives on disk under `%LOCALAPPDATA%` rather than in SQLite
/// (PROJECT_MASTER 6): images are large, rarely queried, and — for the ones
/// that came out of a file — entirely regenerable, so keeping them out of the
/// database keeps it small and its backups cheap.
///
/// The ones a listener chose are *not* regenerable, which is worth knowing
/// before anybody writes a cache-clearing routine.
pub trait ArtworkCachePort: Send + Sync {
    /// Stores an image, replacing any previous one under the same key.
    fn store(&self, cover: CoverOf, image: &[u8]) -> Result<()>;

    /// Where the image lives, if there is one.
    fn path_for(&self, cover: CoverOf) -> Option<PathBuf>;

    /// Where a small copy lives, making one the first time it is asked for.
    ///
    /// A list draws covers at forty-odd pixels and the stored ones are
    /// routinely a thousand square. Decoding the second to draw the first, once
    /// per row, is what makes a library of a few thousand tracks scroll badly —
    /// so the small copy is made once, kept beside the original, and thrown
    /// away with it when the cover changes.
    ///
    /// `None` where there is no cover to shrink, or where the picture could not
    /// be read: a row without a thumbnail draws its fallback, which is a
    /// smaller failure than a row that will not draw at all.
    fn thumbnail_for(&self, cover: CoverOf) -> Option<PathBuf>;

    /// Drops the image. Safe to call when there is none.
    fn remove(&self, cover: CoverOf) -> Result<()>;
}

impl CoverOf {
    /// What to show for a track: the listener's choice, else the file's own.
    ///
    /// The order is the whole rule, and it is the same one the tags follow — a
    /// local override outranks what the file says, and nothing outranks a
    /// deliberate choice except a later deliberate choice.
    pub fn shown_for(
        cache: &dyn ArtworkCachePort,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Option<PathBuf> {
        cache
            .path_for(Self::ChosenTrack(profile_id, media_file_id))
            .or_else(|| cache.path_for(Self::Track(media_file_id)))
    }

    /// The same choice, in the size a list draws.
    ///
    /// Separate from [`Self::shown_for`] rather than a flag on it, because the
    /// precedence has to be applied to the *thumbnails*: asking which full
    /// image wins and then shrinking that one would make a chosen cover with no
    /// thumbnail yet fall back to the file's own, which is the wrong picture
    /// rather than a slower one.
    pub fn thumbnail_shown_for(
        cache: &dyn ArtworkCachePort,
        profile_id: ProfileId,
        media_file_id: MediaFileId,
    ) -> Option<PathBuf> {
        cache
            .thumbnail_for(Self::ChosenTrack(profile_id, media_file_id))
            .or_else(|| cache.thumbnail_for(Self::Track(media_file_id)))
    }
}
