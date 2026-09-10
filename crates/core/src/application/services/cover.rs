//! Choosing a picture, wherever the picture is going.
//!
//! Three services put an image somewhere a listener chose: the library, for a
//! recording whose own tag has none or has a bad one; playlists, for a list; and
//! profiles, for the face beside a name. The flow is the same every time — open
//! a chooser, read what comes back, refuse what is not a picture, store it under
//! a key — and only the key and the event differ.
//!
//! It was written out twice before the third one was needed, which is the point
//! at which a third copy stops being cheaper than a shared one.
//!
//! Free functions rather than a service: there is no state here and nothing to
//! own. A service would be a name wrapped round two calls.

use std::sync::Arc;

use crate::domain::policies::artwork_policy::looks_like_an_image;
use crate::domain::ports::artwork_cache::{ArtworkCachePort, CoverOf};
use crate::domain::ports::file_system::FileSystemPort;
use crate::domain::ports::folder_picker::FolderPickerPort;
use crate::{CoreError, Result};

/// What the three of them need to put a picture somewhere.
#[derive(Clone)]
pub struct CoverPorts {
    /// Where a chosen picture is kept.
    pub artwork: Arc<dyn ArtworkCachePort>,
    /// The chooser it is picked with.
    pub picker: Arc<dyn FolderPickerPort>,
    /// For reading the file that was chosen, and nothing else.
    pub files: Arc<dyn FileSystemPort>,
}

/// Asks for a picture and stores it under `cover`.
///
/// `Ok(false)` means the chooser was closed, which is an answer rather than a
/// failure: a listener who opens the dialog and changes their mind has not done
/// anything wrong, and nothing should be reported to them.
///
/// A file that is not a picture *is* reported, by name. The check is the
/// domain's — four magic numbers, no decoding — because a file that will not
/// decode later is a blank square with no explanation, and the moment to say so
/// is the moment it was chosen.
pub fn choose(ports: &CoverPorts, cover: CoverOf, title: &str) -> Result<bool> {
    let Some(path) = ports.picker.pick_image(title)? else {
        return Ok(false);
    };

    let image = ports.files.read(&path)?;
    if !looks_like_an_image(&image) {
        return Err(CoreError::invalid(
            "cover",
            format!("{} is not a picture this can read", path.display()),
        ));
    }

    ports.artwork.store(cover, &image)?;
    Ok(true)
}

/// Takes a chosen picture back off, leaving whatever stood there before it.
pub fn clear(ports: &CoverPorts, cover: CoverOf) -> Result<()> {
    ports.artwork.remove(cover)
}
