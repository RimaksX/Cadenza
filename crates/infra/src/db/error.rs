//! Turning rusqlite failures into domain errors.

use cadenza_core::CoreError;
use rusqlite::ErrorCode;

/// Maps a rusqlite error onto [`CoreError`].
///
/// Constraint violations become [`CoreError::Conflict`] and everything else
/// becomes [`CoreError::Storage`]. That distinction is the one callers can act
/// on: a conflict means the caller asked for something the data model forbids —
/// a duplicate name, a second queued job for the same file — and is worth
/// reporting to the user, while anything else is a storage fault they cannot fix.
///
/// The rusqlite type never crosses this boundary, which is what keeps
/// `cadenza-core` free of infrastructure types.
pub fn db_error(err: rusqlite::Error) -> CoreError {
    match &err {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::ConstraintViolation =>
        {
            CoreError::Conflict(err.to_string())
        }
        _ => CoreError::Storage(err.to_string()),
    }
}

/// Adds context to a storage failure, naming the operation that failed.
pub fn db_error_in(context: &str) -> impl Fn(rusqlite::Error) -> CoreError + '_ {
    move |err| match db_error(err) {
        CoreError::Conflict(message) => CoreError::Conflict(format!("{context}: {message}")),
        CoreError::Storage(message) => CoreError::Storage(format!("{context}: {message}")),
        other => other,
    }
}
