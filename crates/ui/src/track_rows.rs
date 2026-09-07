//! Rows whose covers are read only when somebody looks at them.
//!
//! Every other list in this window hands Slint a `VecModel` — a vector built
//! whole, handed over, done. That cannot carry covers. A library of five
//! thousand tracks would decode five thousand pictures every time the list was
//! rebuilt, which happens on a search, a scan, a rename and a track finishing,
//! and all but the dozen rows on screen would be thrown away undrawn.
//!
//! `ListView` already asks for the rows it is about to draw and no others. This
//! is the other half of that bargain: a model that answers those questions and
//! does the work in the answering.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use cadenza_core::application::services::LibraryService;
use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::track::TrackSummary;
use slint::{Image, Model, ModelNotify, ModelTracker};

use crate::TrackRowData;
use crate::view_models::library_vm;

/// How many decoded covers are held before the lot is dropped.
///
/// A thumbnail is 96 square, which is about 36 kB decoded; five hundred of them
/// is eighteen megabytes, and a library scrolled end to end without a bound
/// would be a hundred and eighty. Dropping all of them rather than the oldest
/// is deliberate: the cost of being wrong is decoding a picture again, which is
/// a millisecond, and an eviction policy is a thing to get subtly wrong for
/// years.
const KEPT_COVERS: usize = 512;

/// A list of tracks that finds each cover the first time it is drawn.
pub struct TrackRows {
    /// Everything about a row except its cover, built once.
    rows: Vec<TrackRowData>,
    /// Which track each row is, so a cover can be found for it.
    ids: Vec<MediaFileId>,
    library: Arc<LibraryService>,
    covers: RefCell<HashMap<usize, Image>>,
    notify: ModelNotify,
}

impl TrackRows {
    /// Builds the rows, and nothing else. No picture is read here.
    pub fn new(library: Arc<LibraryService>, summaries: &[TrackSummary]) -> Self {
        Self {
            rows: library_vm::rows(summaries),
            ids: summaries
                .iter()
                .map(|summary| summary.media_file_id)
                .collect(),
            library,
            covers: RefCell::new(HashMap::new()),
            notify: ModelNotify::default(),
        }
    }

    /// The cover for one row, read once and then remembered.
    ///
    /// A row with no cover — most of them, in most libraries — costs one failed
    /// lookup per draw and nothing else. It is not worth remembering the
    /// absence: the row draws its fallback either way, and a listener who adds
    /// a cover should see it without restarting.
    fn cover(&self, row: usize) -> Option<Image> {
        if let Some(image) = self.covers.borrow().get(&row) {
            return Some(image.clone());
        }

        let path = self.library.thumbnail_for(*self.ids.get(row)?).ok()??;
        let image = Image::load_from_path(&path).ok()?;

        let mut covers = self.covers.borrow_mut();
        if covers.len() >= KEPT_COVERS {
            covers.clear();
        }
        covers.insert(row, image.clone());

        Some(image)
    }
}

impl Model for TrackRows {
    type Data = TrackRowData;

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let mut data = self.rows.get(row)?.clone();
        if let Some(cover) = self.cover(row) {
            data.cover = cover;
        }
        Some(data)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
