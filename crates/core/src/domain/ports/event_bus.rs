//! Broadcasting that something changed.

use crate::domain::ids::{MediaFileId, ProfileId};

/// Something the rest of the application should know about.
///
/// The variants are deliberately coarse. A fine-grained event per field would
/// mean inventing thirty variants now and keeping every one of them in sync with
/// the view state forever; instead an event says *what area* changed and the
/// subscriber re-reads the state it cares about. The UI is a function of view
/// state anyway, so it has nothing to gain from deltas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    /// The active profile changed. Everything profile-scoped is now stale.
    ProfileSwitched(ProfileId),
    /// Transport state, position, volume or the current track moved.
    PlaybackChanged,
    /// The queue's contents or ordering changed.
    QueueChanged,
    /// Tracks were added to or removed from the library.
    LibraryChanged,
    /// A playlist was created, edited or deleted.
    PlaylistsChanged,
    /// Equaliser settings or the selected preset changed.
    EqChanged,
    /// Background analysis finished for one file.
    AnalysisCompleted(MediaFileId),
    /// A file needs a decision in the review queue.
    ReviewPending,
}

/// A subscriber callback.
///
/// Called on whichever thread published the event, so handlers must be cheap and
/// must never block. Anything expensive belongs on the subscriber's own thread.
pub type EventHandler = Box<dyn Fn(&DomainEvent) + Send + Sync>;

/// Fan-out of domain events to interested parties.
pub trait EventBusPort: Send + Sync {
    /// Notifies every subscriber. Never called from the audio thread.
    fn publish(&self, event: DomainEvent);

    /// Registers a handler for every subsequent event.
    fn subscribe(&self, handler: EventHandler);
}
