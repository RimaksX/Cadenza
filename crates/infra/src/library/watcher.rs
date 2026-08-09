//! Noticing that files changed underneath us.
//!
//! Two pieces: a translation from what `notify` reports into what Cadenza cares
//! about, and a debouncer that waits for a burst to settle. Both are pure and
//! take the current instant as an argument, so the tests below assert on exact
//! boundaries instead of sleeping and hoping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cadenza_core::domain::ports::file_watcher::{
    DEBOUNCE, FileChange, FileChangeHandler, FileWatcherPort,
};
use cadenza_core::{CoreError, Result};
use notify::EventKind;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// How often the debouncer looks for changes that have settled.
///
/// Short enough that the wait never feels much longer than [`DEBOUNCE`], long
/// enough that an idle Cadenza is not waking up hundreds of times a second.
const TICK: Duration = Duration::from_millis(100);

/// Collects filesystem events and releases them once they stop arriving.
///
/// Copying an album into a watched folder produces several events per file: a
/// create, then a modify for each write. Acting on the first one would mean
/// reading a file that is still being written. Keyed by path, so a burst
/// against one file collapses into a single change.
#[derive(Debug, Default)]
pub struct Debouncer {
    pending: HashMap<PathBuf, (FileChange, Instant)>,
}

impl Debouncer {
    /// An empty debouncer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a change, restarting its wait.
    ///
    /// The newest event for a path wins: a file created and then deleted before
    /// anything settled is a deletion, and reporting the creation first would
    /// send the scanner after a file that is no longer there.
    pub fn record(&mut self, change: FileChange, at: Instant) {
        self.pending.insert(key_of(&change), (change, at));
    }

    /// Takes every change that has been quiet for [`DEBOUNCE`].
    pub fn take_ready(&mut self, now: Instant) -> Vec<FileChange> {
        let window = Duration::from_millis(DEBOUNCE.as_millis());

        let ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, (_, seen))| now.duration_since(*seen) >= window)
            .map(|(path, _)| path.clone())
            .collect();

        ready
            .into_iter()
            .filter_map(|path| self.pending.remove(&path).map(|(change, _)| change))
            .collect()
    }

    /// How many changes are still waiting.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.pending.len()
    }
}

/// The path a change is keyed by.
///
/// A rename is keyed by where the file ended up: that is the path anything
/// downstream will look at.
fn key_of(change: &FileChange) -> PathBuf {
    match change {
        FileChange::Created(path)
        | FileChange::Modified(path)
        | FileChange::Removed(path)
        | FileChange::Renamed { to: path, .. } => path.clone(),
    }
}

/// Translates a `notify` event into the changes Cadenza acts on.
///
/// Returns nothing for the events that carry no information for a music library
/// — access times, metadata-only touches, and whatever a platform reports that
/// does not map onto "this file appeared, changed, moved or went away".
#[must_use]
pub fn classify(kind: &EventKind, paths: &[PathBuf]) -> Option<FileChange> {
    let first = paths.first()?.clone();

    match kind {
        EventKind::Create(CreateKind::File | CreateKind::Any) => Some(FileChange::Created(first)),
        EventKind::Remove(RemoveKind::File | RemoveKind::Any) => Some(FileChange::Removed(first)),

        // A rename reported as one event: keep it whole, so the library can move
        // the row instead of losing the track's history and playlist membership
        // to a delete followed by an insert.
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() >= 2 => {
            Some(FileChange::Renamed {
                from: first,
                to: paths[1].clone(),
            })
        }
        // Platforms that report the two halves separately.
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => Some(FileChange::Removed(first)),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => Some(FileChange::Created(first)),

        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any) => {
            Some(FileChange::Modified(first))
        }

        _ => None,
    }
}

/// Watches library folders using the platform's own facility.
pub struct NotifyFileWatcher {
    watcher: Mutex<RecommendedWatcher>,
    handler: Arc<RwLock<Option<FileChangeHandler>>>,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl NotifyFileWatcher {
    /// Starts watching. Events are delivered once they have settled.
    pub fn new() -> Result<Self> {
        let debouncer = Arc::new(Mutex::new(Debouncer::new()));
        let handler: Arc<RwLock<Option<FileChangeHandler>>> = Arc::new(RwLock::new(None));
        let stop = Arc::new(AtomicBool::new(false));

        let collector = Arc::clone(&debouncer);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // A dropped event is a missed change, not a crash. The next scan
            // picks it up, which is why scanning exists as well as watching.
            let Ok(event) = event else {
                return;
            };
            if let Some(change) = classify(&event.kind, &event.paths) {
                collector
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .record(change, Instant::now());
            }
        })
        .map_err(|err| CoreError::FileSystem(format!("could not start watching files: {err}")))?;

        let worker = thread::Builder::new()
            .name("cadenza-file-watcher".to_owned())
            .spawn({
                let debouncer = Arc::clone(&debouncer);
                let handler = Arc::clone(&handler);
                let stop = Arc::clone(&stop);
                move || flush_loop(&debouncer, &handler, &stop)
            })
            .map_err(|err| {
                CoreError::FileSystem(format!("could not start the file watcher thread: {err}"))
            })?;

        Ok(Self {
            watcher: Mutex::new(watcher),
            handler,
            stop,
            worker: Mutex::new(Some(worker)),
        })
    }
}

/// Releases settled changes to the handler until asked to stop.
fn flush_loop(
    debouncer: &Arc<Mutex<Debouncer>>,
    handler: &Arc<RwLock<Option<FileChangeHandler>>>,
    stop: &Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(TICK);

        // The lock is released before the handler runs: a handler that rescans a
        // folder can take a while, and holding the debouncer through it would
        // stall every event behind it.
        let ready = debouncer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_ready(Instant::now());

        if ready.is_empty() {
            continue;
        }

        let installed = handler.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(callback) = installed.as_ref() {
            for change in ready {
                callback(change);
            }
        }
    }
}

impl FileWatcherPort for NotifyFileWatcher {
    fn watch(&self, path: &Path, recursive: bool) -> Result<()> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        self.watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .watch(path, mode)
            .map_err(|err| {
                CoreError::FileSystem(format!("could not watch {}: {err}", path.display()))
            })
    }

    fn unwatch(&self, path: &Path) -> Result<()> {
        self.watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwatch(path)
            .map_err(|err| {
                CoreError::FileSystem(format!("could not stop watching {}: {err}", path.display()))
            })
    }

    fn set_handler(&self, handler: FileChangeHandler) {
        *self.handler.write().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }
}

impl Drop for NotifyFileWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // Joining rather than detaching: the handler holds references to the
            // repositories, and letting it run against a half-dropped
            // application is how a shutdown turns into a crash report.
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{Debouncer, classify};
    use cadenza_core::domain::ports::file_watcher::{DEBOUNCE, FileChange};
    use notify::EventKind;
    use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

    fn window() -> Duration {
        Duration::from_millis(DEBOUNCE.as_millis())
    }

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("D:/Music/{name}"))
    }

    #[test]
    fn a_change_is_held_until_it_stops_arriving() {
        let mut debouncer = Debouncer::new();
        let start = Instant::now();

        debouncer.record(FileChange::Created(path("a.flac")), start);

        assert!(
            debouncer.take_ready(start).is_empty(),
            "acting immediately would read a file still being written"
        );
        assert_eq!(debouncer.waiting(), 1);

        let ready = debouncer.take_ready(start + window());
        assert_eq!(ready, vec![FileChange::Created(path("a.flac"))]);
        assert_eq!(debouncer.waiting(), 0);
    }

    #[test]
    fn a_burst_against_one_file_collapses_into_one_change() {
        let mut debouncer = Debouncer::new();
        let start = Instant::now();

        // What copying a file actually looks like: a create, then writes.
        debouncer.record(FileChange::Created(path("a.flac")), start);
        debouncer.record(FileChange::Modified(path("a.flac")), start + window() / 2);
        debouncer.record(FileChange::Modified(path("a.flac")), start + window());

        assert!(
            debouncer.take_ready(start + window()).is_empty(),
            "each event restarts the wait"
        );

        let ready = debouncer.take_ready(start + window() * 2);
        assert_eq!(ready.len(), 1, "one file, one change");
    }

    #[test]
    fn the_last_word_wins() {
        let mut debouncer = Debouncer::new();
        let start = Instant::now();

        debouncer.record(FileChange::Created(path("a.flac")), start);
        debouncer.record(FileChange::Removed(path("a.flac")), start);

        assert_eq!(
            debouncer.take_ready(start + window()),
            vec![FileChange::Removed(path("a.flac"))],
            "a file created and deleted before settling is a deletion"
        );
    }

    #[test]
    fn separate_files_settle_separately() {
        let mut debouncer = Debouncer::new();
        let start = Instant::now();

        debouncer.record(FileChange::Created(path("a.flac")), start);
        debouncer.record(FileChange::Created(path("b.flac")), start + window());

        let first = debouncer.take_ready(start + window());
        assert_eq!(first, vec![FileChange::Created(path("a.flac"))]);
        assert_eq!(debouncer.waiting(), 1, "the second is still settling");
    }

    #[test]
    fn a_rename_is_keyed_by_where_the_file_ended_up() {
        let mut debouncer = Debouncer::new();
        let start = Instant::now();

        debouncer.record(
            FileChange::Renamed {
                from: path("old.flac"),
                to: path("new.flac"),
            },
            start,
        );
        debouncer.record(FileChange::Modified(path("new.flac")), start);

        assert_eq!(
            debouncer.waiting(),
            1,
            "both events concern the same file after the move"
        );
    }

    #[test]
    fn the_events_that_matter_are_recognised() {
        assert_eq!(
            classify(&EventKind::Create(CreateKind::File), &[path("a.flac")]),
            Some(FileChange::Created(path("a.flac")))
        );
        assert_eq!(
            classify(&EventKind::Remove(RemoveKind::File), &[path("a.flac")]),
            Some(FileChange::Removed(path("a.flac")))
        );
        assert_eq!(
            classify(
                &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
                &[path("a.flac")]
            ),
            Some(FileChange::Modified(path("a.flac")))
        );
    }

    #[test]
    fn a_rename_stays_whole_when_the_platform_reports_it_whole() {
        assert_eq!(
            classify(
                &EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                &[path("old.flac"), path("new.flac")]
            ),
            Some(FileChange::Renamed {
                from: path("old.flac"),
                to: path("new.flac"),
            }),
            "a move must not become a delete plus an insert, which would lose \
             the track's history and playlist membership"
        );
    }

    #[test]
    fn a_split_rename_becomes_a_removal_and_a_creation() {
        assert_eq!(
            classify(
                &EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                &[path("old.flac")]
            ),
            Some(FileChange::Removed(path("old.flac")))
        );
        assert_eq!(
            classify(
                &EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                &[path("new.flac")]
            ),
            Some(FileChange::Created(path("new.flac")))
        );
    }

    #[test]
    fn noise_is_ignored() {
        assert_eq!(
            classify(
                &EventKind::Access(notify::event::AccessKind::Read),
                &[path("a.flac")]
            ),
            None,
            "opening a file to play it is not a library change"
        );
        assert_eq!(
            classify(&EventKind::Create(CreateKind::File), &[]),
            None,
            "an event with no path says nothing"
        );
    }
}
