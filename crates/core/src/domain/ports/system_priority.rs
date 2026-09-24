//! Keeping background work out of the way.

use crate::Result;

/// How much of the machine a thread is entitled to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityClass {
    /// Yields to everything else. Used by scanning, hashing and feature
    /// extraction so that they stay under the background CPU budget and never
    /// compete with playback or with whatever else the listener is doing.
    Background,
    /// Ordinary application priority.
    Normal,
}

/// Adjusts thread scheduling priority.
pub trait SystemPriorityPort: Send + Sync {
    /// Sets the priority of the calling thread.
    ///
    /// Best-effort: an implementation that cannot lower priority on the current
    /// platform reports success and leaves scheduling alone, because failing to
    /// deprioritise a worker is a performance problem, not a correctness one.
    fn set_current_thread(&self, class: PriorityClass) -> Result<()>;
}
