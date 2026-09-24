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

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use cadenza_core::application::services::LibraryService;
use cadenza_core::domain::ids::MediaFileId;
use cadenza_core::domain::track::TrackSummary;
use slint::{Image, Model, ModelNotify, ModelRc, ModelTracker};

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

/// Every cover this window has decoded, by the track it belongs to.
///
/// Shared rather than owned, and keyed by the track rather than by the row it
/// happens to be on. A model is built fresh for every change to the library —
/// one track added, one search typed — and a cache inside it would be thrown
/// away with it, so adding a song would cost the decoding of every cover on
/// screen.
pub type Covers = Rc<RefCell<HashMap<MediaFileId, Image>>>;

/// A list of tracks that finds each cover the first time it is drawn.
pub struct TrackRows {
    /// Everything about a row except its cover, built once.
    rows: Vec<TrackRowData>,
    /// Which track each row is, so a cover can be found for it.
    ids: Vec<MediaFileId>,
    library: Arc<LibraryService>,
    covers: Covers,
    /// The picked tracks of the list this is, for the lists that pick.
    picked: Option<Rc<RefCell<HashSet<MediaFileId>>>>,
    notify: ModelNotify,
}

impl TrackRows {
    /// Builds the rows, and nothing else. No picture is read here.
    pub fn new(library: Arc<LibraryService>, covers: Covers, summaries: &[TrackSummary]) -> Self {
        Self {
            rows: library_vm::rows(summaries),
            ids: summaries
                .iter()
                .map(|summary| summary.media_file_id)
                .collect(),
            library,
            covers,
            picked: None,
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
        let id = *self.ids.get(row)?;
        if let Some(image) = self.covers.borrow().get(&id) {
            return Some(image.clone());
        }

        let path = self.library.thumbnail_for(id).ok()??;
        let image = Image::load_from_path(&path).ok()?;

        let mut covers = self.covers.borrow_mut();
        if covers.len() >= KEPT_COVERS {
            covers.clear();
        }
        covers.insert(id, image.clone());

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
        data.selected = self
            .picked
            .as_ref()
            .is_some_and(|picked| picked.borrow().contains(&self.ids[row]));
        if let Some(cover) = self.cover(row) {
            data.cover = cover;
        }
        Some(data)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// Which rows of one list are picked, by the track on them - so a pick
/// survives the list being drawn again - and the row the last single pick was
/// made on, for a range to run from.
#[derive(Default)]
pub struct Picking {
    picked: Rc<RefCell<HashSet<MediaFileId>>>,
    anchor: Cell<Option<usize>>,
    rows: RefCell<Option<Rc<TrackRows>>>,
}

impl Picking {
    /// The model for `rows`, answering which of them are picked. A pick of a
    /// track the list no longer shows is let go.
    pub fn model(&self, mut rows: TrackRows) -> ModelRc<TrackRowData> {
        self.picked.borrow_mut().retain(|id| rows.ids.contains(id));
        rows.picked = Some(Rc::clone(&self.picked));
        let rows = Rc::new(rows);
        *self.rows.borrow_mut() = Some(Rc::clone(&rows));
        ModelRc::from(rows)
    }

    /// A row picked or let go: `add` keeps the others (the control key),
    /// `range` takes every row from the last single pick to this one (shift).
    pub fn pick(&self, row: usize, add: bool, range: bool) {
        let Some(rows) = self.rows.borrow().clone() else {
            return;
        };
        let Some(&id) = rows.ids.get(row) else {
            return;
        };
        {
            let mut picked = self.picked.borrow_mut();
            match (range, self.anchor.get()) {
                (true, Some(anchor)) if anchor < rows.ids.len() => {
                    if !add {
                        picked.clear();
                    }
                    let (first, last) = (anchor.min(row), anchor.max(row));
                    picked.extend(rows.ids[first..=last].iter().copied());
                }
                _ => {
                    if !picked.remove(&id) {
                        picked.insert(id);
                    }
                    self.anchor.set(Some(row));
                }
            }
        }
        rows.redraw();
    }

    /// Lets every row go.
    pub fn clear(&self) {
        self.picked.borrow_mut().clear();
        self.anchor.set(None);
        if let Some(rows) = self.rows.borrow().as_ref() {
            rows.redraw();
        }
    }

    /// How many are picked.
    pub fn count(&self) -> usize {
        self.picked.borrow().len()
    }

    /// The picked rows, in the order the list shows them, with their tracks.
    pub fn picked(&self) -> Vec<(usize, MediaFileId)> {
        let picked = self.picked.borrow();
        self.rows.borrow().as_ref().map_or_else(Vec::new, |rows| {
            rows.ids
                .iter()
                .enumerate()
                .filter(|(_, id)| picked.contains(id))
                .map(|(row, id)| (row, *id))
                .collect()
        })
    }
}

impl TrackRows {
    /// Every row asked again, for what is picked.
    fn redraw(&self) {
        for row in 0..self.rows.len() {
            self.notify.row_changed(row);
        }
    }
}
