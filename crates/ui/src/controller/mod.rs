//! What happens when the listener does something.
//!
//! The controller owns the direction of travel: properties down into the
//! window, commands up into the application layer. It holds no rules — every
//! method here is a translation and a call.

mod eq;
mod fetch;
mod home;
mod library;
mod picking;
mod player;
mod playlists;
mod profile;
mod queue;
mod radio;
mod settings;
mod shelves;
mod window;

pub use picking::PickedIn;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use cadenza_core::application::services::Fetched;
use cadenza_core::application::view_state::Continuation;
use cadenza_core::domain::eq::{EqBand, EqMode};
use cadenza_core::domain::ids::{
    EqPresetId, ImportReviewId, MediaFileId, MoodId, PlaylistId, ProfileId,
};
use cadenza_core::domain::playback::SleepTimer;
use cadenza_core::domain::policies::eq_policy::{MAX_BAND_HZ, MAX_BAND_Q, MIN_BAND_HZ, MIN_BAND_Q};
use cadenza_core::domain::policies::link_policy::readings_of;
use cadenza_core::domain::ports::fetcher::FetchWhat;
use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::queue::{QueueOrigin, RepeatMode};
use cadenza_core::domain::radio::{MIN_BATCH_SIZE, RadioFeedback};
use cadenza_core::domain::review::ReviewResolution;
use cadenza_core::domain::settings::{
    CrossfadeDuration, InterfaceScale, Language, PlaybackSpeed, ProfileFolder, SideColumn,
    WindowPlacement,
};
use cadenza_core::domain::track::TrackSummary;
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;
use cadenza_core::domain::value_objects::{DurationMs, GainDb, PlaybackPosition, Volume};
use cadenza_core::{CoreError, Result};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::track_rows::{Covers, Picking, TrackRows};
use crate::view_models::library_vm::Order;
use crate::view_models::{
    eq_vm, library_vm, player_vm, playlist_vm, radio_vm, review_vm, stats_vm,
};
use crate::{
    AppWindow, EqBandData, Equaliser, Fetch, FolderRowData, Home, Library, Listening, MenuItemData,
    MoodRowData, Player, PlaylistCardData, Playlists, Queue, Radio, ReviewChoice, ReviewRowData,
    Section, Settings, Shelves, TakenOutRowData, Theme, ToneBand, TopTrackData, Transfer,
    UiServices,
};

/// How many of the tracks heard lately Home lists, and of the quiet ones.
const HOME_RECENT: usize = 5;

/// How much of what plays on after the queue Home's panel shows.
const QUEUE_THEN: usize = 10;

/// How many covers a row on Home holds at most: as many as fit are shown.
const HOME_TILES: usize = 6;

/// A place in a list of `len`, by chance, or nothing in an empty one.
///
/// The standard library's random hash keys as the die: seeded from the
/// system once, and moved on at every `RandomState`. The clock's nanoseconds
/// were the die before, and on Windows they step by a hundred, so every list
/// of a length dividing a hundred started at its first track.
///
/// ponytail: good enough for "play me something"; a real generator is a
/// dependency for a button.
fn pick(len: usize) -> Option<usize> {
    use std::hash::BuildHasher;
    let roll = std::collections::hash_map::RandomState::new().hash_one(());
    (len > 0).then(|| usize::try_from(roll % len as u64).unwrap_or(0))
}

/// One of `items`, by chance.
fn chance<T>(items: &[T]) -> Option<&T> {
    pick(items.len()).map(|index| &items[index])
}

/// What to do when there is a profile but nothing in it.
const NO_TRACKS_HINT: &str =
    "drop a file on this window, paste a link above,\nor point Cadenza at a folder in Settings";

/// What to do when there are no playlists.
const NO_PLAYLISTS_HINT: &str = "press + NEW PLAYLIST to start one";

/// What to do when a search matches nothing.
const NO_MATCH_HINT: &str = "no track here answers to that";

/// What to do when a playlist has nothing in it.
const EMPTY_PLAYLIST_HINT: &str = "add tracks from the library\nwith the ··· at the end of a row";

/// What the one offered button would do, when there is one.
///
/// Two things go wrong in a way a listener can put right — the programs are
/// not installed, and the installed one has fallen behind — and both are
/// answered by pressing once. One button rather than two, because only one of
/// them is ever true at a time and a row of buttons that are usually both
/// wrong teaches people to read none of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Offer {
    Install,
    Update,
}

impl Offer {
    /// What the button says.
    fn label(self) -> &'static str {
        match self {
            Self::Install => crate::text::tr("INSTALL"),
            Self::Update => crate::text::tr("UPDATE"),
        }
    }
}

/// What a fetch running on another thread has got to.
///
/// Shared with that thread and read by the tick, which is the same arrangement
/// the watcher uses and for the same reason: the window is not `Send`, nothing
/// off the event loop may touch it, and a flag read four times a second is
/// cheaper than any way of asking to be let in.
#[derive(Default)]
struct Fetching {
    /// True from the press until the thread has finished.
    running: AtomicBool,
    /// How far the downloader says it has got, on the track it is on.
    percent: AtomicU8,
    /// Which track of how many, where a playlist is coming. Zero for neither.
    item: AtomicU32,
    of: AtomicU32,
    /// Set when the listener presses stop, read by the thread between lines.
    stopping: AtomicBool,
    /// Set once, at the end.
    finished: Mutex<Option<Ended>>,
}

/// How a fetch turned out, in the words the head will show.
///
/// Three cases rather than a string and a flag: only one of them offers the
/// listener something to press, and the window should not have to read the
/// sentence to work out which.
enum Ended {
    /// It is in the library.
    Landed(String),
    /// It did not work, and one press would fix it.
    Offer(String, Offer),
    /// The fixing is done: what it said, and the thing to try again.
    Fixed(String),
    /// There was nowhere to put it, and Cadenza can fix that.
    NeedsFolder(String),
    /// Anything else: a bad link, a missing program, a refusal at the far end.
    Failed(String),
}

/// Holds the services and pushes state into the window.
pub struct Controller {
    services: UiServices,
    window: Weak<AppWindow>,
    /// The active profile, kept because the theme belongs to it.
    profile: RefCell<Option<Profile>>,
    /// What the library is being filtered by, if anything.
    query: RefCell<String>,
    /// Every cover this window has decoded, kept across the models that come
    /// and go. Adding one track used to cost the decoding of every cover on
    /// screen, because the cache lived inside a model that was thrown away.
    covers: Covers,
    /// Pictures that are not a track's: playlist covers, and the faces beside
    /// names. Keyed by where they came from, because that is what identifies
    /// the bytes.
    ///
    /// Without it, entering the playlists page decoded every cover again —
    /// **measured at 334 ms of a 335 ms refresh**, all of it one 1000-pixel
    /// WebP, every single time the page was opened.
    pictures: RefCell<HashMap<PathBuf, slint::Image>>,
    /// The colour of each of those pictures, worked out once: it is asked on
    /// every change to the list it heads, and working it out copies the
    /// whole picture.
    tints: RefCell<HashMap<PathBuf, Option<slint::Color>>>,
    /// What the offered button would do, while one is offered.
    offer: Cell<Option<Offer>>,
    /// What the last press asked for, so that fixing the reason it failed can
    /// then do the thing that failed.
    asked_for: Cell<FetchWhat>,
    /// The library as it was last read, which is what a search filters.
    ///
    /// Kept because searching used to read the whole table again for every
    /// character typed. Measured at five thousand tracks — the top of the size
    /// the player is built for — that read is eight milliseconds of the fifteen
    /// a keystroke costs, and it is the eight that buys nothing: the library
    /// cannot have changed between two letters.
    ///
    /// Every path that could have changed it goes through
    /// [`Self::refresh_library`], which reads and replaces this. Nothing else
    /// writes it.
    shown_library: RefCell<Vec<TrackSummary>>,
    /// The library's rows in the order they are drawn, searched and sorted as
    /// they are on screen, so the tick can find the playing one without
    /// searching and sorting again. Written wherever the list is drawn.
    shown_order: RefCell<Vec<MediaFileId>>,
    /// Whether the track playing is a favourite, and which track that answer
    /// is about. The tick asks four times a second and the answer is a read of
    /// the whole favourites list, so it is kept until something could have
    /// changed it: any command, or a track ending (which is what earns a
    /// place).
    favourite: RefCell<Option<(String, bool)>>,
    /// The rows picked in the library, and in the open playlist.
    library_picking: Picking,
    playlist_picking: Picking,
    /// The artist open on their own page.
    open_artist: Cell<Option<cadenza_core::domain::ids::ArtistId>>,
    /// The page on screen, as far as the window has said: what a change to
    /// the library has to redraw besides the library itself.
    shown: Cell<Section>,
    /// How many carries had begun when the button last came up.
    carries_seen: Cell<i32>,
    /// Whose cover the player bar is showing, so it is read from disk when the
    /// track changes rather than four times a second.
    showing_cover: RefCell<String>,
    /// Whose waveform the bar is showing, and whether it has it yet. Until it
    /// has, the store is asked on each tick - a point read - for a shape the
    /// background thread is still measuring.
    showing_wave: RefCell<(String, bool)>,
    /// The playlist whose page is open, if one is.
    ///
    /// Held because playing a track from a playlist has to say which playlist:
    /// the queue's entries carry it, and that is what makes the rest of the
    /// list follow rather than the rest of the library.
    open_playlist: RefCell<Option<PlaylistId>>,
    /// The bands the equaliser screen is showing.
    ///
    /// Held rather than rebuilt. Handing the window a *new* model makes Slint
    /// destroy and recreate every element the repeater built from it — and one
    /// of those is the touch area holding the pointer during a drag. That is
    /// why a fader could only ever be clicked: the first movement threw away
    /// the target that was following it.
    eq_bands: Rc<VecModel<EqBandData>>,
    /// Which bell the equaliser's numbers are about.
    ///
    /// Interface state and nothing else: which band is being looked at changes
    /// nothing about the sound, so nothing outside the window needs telling.
    selected_band: Cell<usize>,
    /// The size the window has been given room for.
    ///
    /// The window is created before anybody knows whose it is, so it opens at
    /// the size for 100 per cent; the profile's own size arrives a moment
    /// later. Remembering which one the frame was built for is what lets the
    /// difference be made up exactly once.
    sized_for: Cell<f32>,
    /// The fetch in progress, if there is one.
    fetching: Arc<Fetching>,
    /// The order the library is shown in.
    ///
    /// Interface state, like the search query beside it: which way a list is
    /// turned is nobody's business but the window's, and it is not worth a
    /// column in the database until somebody asks for it to be remembered
    /// between runs.
    order: Cell<Order>,
}

impl Controller {
    /// Binds the controller to a window that has not been shown yet.
    pub fn new(services: UiServices, window: Weak<AppWindow>) -> Self {
        let profile = RefCell::new(services.profile.clone());
        Self {
            services,
            window,
            profile,
            query: RefCell::new(String::new()),
            covers: Covers::default(),
            pictures: RefCell::new(HashMap::new()),
            tints: RefCell::new(HashMap::new()),
            offer: Cell::new(None),
            asked_for: Cell::new(FetchWhat::OneTrack),
            shown_library: RefCell::new(Vec::new()),
            shown_order: RefCell::new(Vec::new()),
            favourite: RefCell::new(None),
            library_picking: Picking::default(),
            playlist_picking: Picking::default(),
            open_artist: Cell::new(None),
            shown: Cell::new(Section::Home),
            carries_seen: Cell::new(0),
            showing_cover: RefCell::new(String::new()),
            showing_wave: RefCell::new((String::new(), false)),
            open_playlist: RefCell::new(None),
            eq_bands: Rc::new(VecModel::default()),
            selected_band: Cell::new(0),
            sized_for: Cell::new(1.0),
            fetching: Arc::new(Fetching::default()),
            order: Cell::new(Order::default()),
        }
    }

    /// Fills every property from scratch.
    pub fn refresh_all(&self) {
        self.refresh_profile();
        // Before anything is drawn: every string below is read in whatever
        // language this leaves selected.
        self.refresh_language();
        // After the profile: which size is chosen belongs to whoever is
        // listening.
        self.refresh_interface_scale();
        self.refresh_folds();
        self.refresh_library();
        self.refresh_queue();
        // Before the lists are drawn, so the favourites list is in them and is
        // current: a month of listening may have happened in another session,
        // or in this one before an upgrade brought the list into existence.
        self.refresh_favourites();
        self.refresh_playlists();
        self.refresh_home();
        self.refresh_eq();
        self.refresh_settings();
        self.refresh_radio();
        self.refresh_listening();
        self.refresh_reviews();
        self.refresh_player();
    }

    /// Runs a command, reports what it says, and refreshes the transport.
    fn run(&self, command: impl FnOnce() -> Result<()>) {
        // Whatever it was may have put the playing track in the favourites or
        // taken it out.
        self.favourite.take();
        match command() {
            Ok(()) => self.clear_message(),
            Err(err) => self.report(&err),
        }
        self.refresh_player();
    }

    /// Puts a failure where the listener will see it.
    ///
    /// The player bar rather than a dialog: nothing here is fatal, and a modal
    /// over a missing file would be a worse interruption than the silence
    /// already is.
    fn report(&self, err: &CoreError) {
        if let Some(window) = self.window.upgrade() {
            show_message(&window, &err.to_string(), true);
        }
    }

    /// Puts a line of news where the listener will see it: the same place a
    /// failure goes, until the next command.
    pub(super) fn say(&self, said: &str) {
        if let Some(window) = self.window.upgrade() {
            show_message(&window, said, false);
        }
    }

    fn clear_message(&self) {
        if let Some(window) = self.window.upgrade() {
            show_message(&window, "", false);
        }
    }
}

/// How long news stays up.
const NEWS_LASTS: std::time::Duration = std::time::Duration::from_secs(4);

thread_local! {
    /// The clock on the news that is up. One, restarted by each new line, so
    /// the clock on one piece of news cannot take away whatever replaced it;
    /// only the event loop's thread shows anything.
    static NEWS: slint::Timer = slint::Timer::default();
}

/// Puts a line where the listener will see it, or takes it away with `""`.
///
/// News goes by itself after a few seconds: it has been read, or it did not
/// matter. A failure stays - somebody may be across the room - until the next
/// command succeeds or it is pressed away.
pub(crate) fn show_message(window: &AppWindow, said: &str, failed: bool) {
    window.set_message_is_error(failed);
    window.set_message(said.into());
    NEWS.with(|clock| {
        if failed || said.is_empty() {
            clock.stop();
            return;
        }
        let window = window.as_weak();
        clock.start(slint::TimerMode::SingleShot, NEWS_LASTS, move || {
            if let Some(window) = window.upgrade() {
                window.set_message(SharedString::new());
            }
        });
    });
}

/// A count for the markup, which counts in `i32`.
fn count(picking: &Picking) -> i32 {
    i32::try_from(picking.count()).unwrap_or(i32::MAX)
}

/// The colour a picture is, for tinting what stands around it, or `None` for
/// one with no colour worth taking.
///
/// A grid of 64 by 64 across the whole picture rather than every pixel. A
/// stride through the flat buffer lands on the same few columns of every row,
/// and on a large cover it read the left edge alone.
fn tint_of(picture: &slint::Image) -> Option<slint::Color> {
    const GRID: u32 = 64;
    let pixels = picture.to_rgba8()?;
    let (width, height) = (pixels.width(), pixels.height());
    let all = pixels.as_slice();
    let mut sample = Vec::new();
    for row in 0..GRID.min(height) {
        for column in 0..GRID.min(width) {
            let x = column * width / GRID.min(width);
            let y = row * height / GRID.min(height);
            let p = all[(y * width + x) as usize];
            sample.push((p.r, p.g, p.b));
        }
    }
    player_vm::tint(&sample).map(|(r, g, b)| slint::Color::from_rgb_u8(r, g, b))
}

/// What [`tint_of`] makes of the photograph the favourites stand on until
/// somebody chooses a picture: measured on the shipped file, because the
/// markup embeds it and Rust never decodes it.
const FAVOURITES_PHOTO_TINT: (u8, u8, u8) = (74, 101, 158);

/// [`FAVOURITES_PHOTO_TINT`], as the colour it is.
fn favourites_photo_tint() -> Option<slint::Color> {
    let (r, g, b) = FAVOURITES_PHOTO_TINT;
    Some(slint::Color::from_rgb_u8(r, g, b))
}

#[cfg(test)]
mod pick_tests {
    use super::pick;

    // A list of four is the case the clock's nanoseconds always started at
    // its first; two hundred draws miss one of four places about once in
    // 10^25.
    #[test]
    fn every_place_in_a_short_list_comes_up() {
        let seen: std::collections::HashSet<usize> = (0..200).filter_map(|_| pick(4)).collect();
        assert_eq!(seen.len(), 4);
        assert_eq!(pick(0), None);
    }
}
