//! Where something that nobody can be told about goes.
//!
//! Cadenza reports what it can to the listener: a command that fails puts a
//! sentence in the player bar. This is for everything that cannot reach them —
//! a watcher thread that could not read one file, a listen that would not
//! record, a queue that failed to save. Those are deliberate silences
//! , and a silence with nowhere to write is a defect
//! nobody can diagnose afterwards.
//!
//! Not a general logging facility, and deliberately not: there is no level
//! anybody turns on, no module filter, no format string. What is written is
//! what could not be said out loud.

/// How much it matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Something happened worth knowing the time of: a start, a switch.
    Info,
    /// Something failed and the application carried on without it.
    Warn,
    /// Something failed that the listener would have wanted to know about.
    Error,
}

impl LogLevel {
    /// The word written in the line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Somewhere to write a line that no window will show.
pub trait LogPort: Send + Sync {
    /// Writes one line. Never fails: whatever went wrong here, the thing that
    /// was being reported is still the more important of the two.
    fn write(&self, level: LogLevel, message: &str);
}

/// A log that keeps nothing.
///
/// What a context has until somebody gives it somewhere to write — a test, a
/// command line that prints its own errors. It exists so that the call sites
/// can be written once and unconditionally: a service that has to ask whether
/// there is a log is a service that will forget to.
pub struct NoLog;

impl LogPort for NoLog {
    fn write(&self, _level: LogLevel, _message: &str) {}
}
