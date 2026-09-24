//! The single error type crossing core's boundaries.

use thiserror::Error;

/// Result alias used throughout the core crate.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Errors produced by the domain and application layers.
///
/// Infrastructure errors (rusqlite, cpal, symphonia, io) are flattened into the
/// string-carrying variants below. That keeps core free of infrastructure types,
/// which is what makes the layering rule enforceable at compile time.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A value failed validation before an entity could be constructed.
    #[error("invalid {field}: {reason}")]
    Invalid {
        /// Name of the offending field or value object.
        field: &'static str,
        /// Human-readable explanation.
        reason: String,
    },

    /// A lookup by identifier found nothing.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind, e.g. `"profile"`.
        entity: &'static str,
        /// Identifier that was looked up.
        id: String,
    },

    /// The operation would violate a uniqueness or state constraint.
    #[error("conflict: {0}")]
    Conflict(String),

    /// An operation needing a profile was attempted while none was active.
    #[error("no active profile")]
    NoActiveProfile,

    /// The storage adapter failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// The audio engine failed.
    #[error("audio error: {0}")]
    Audio(String),

    /// Decoding failed or the format is unsupported.
    #[error("decode error: {0}")]
    Decode(String),

    /// Reading tags or artwork failed.
    #[error("metadata error: {0}")]
    Metadata(String),

    /// A filesystem operation failed.
    #[error("filesystem error: {0}")]
    FileSystem(String),

    /// Background analysis failed for this item.
    #[error("analysis error: {0}")]
    Analysis(String),

    /// The caller cancelled the operation.
    #[error("cancelled")]
    Cancelled,
}

impl CoreError {
    /// Builds an [`CoreError::Invalid`].
    pub fn invalid(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            reason: reason.into(),
        }
    }

    /// Builds a [`CoreError::NotFound`].
    pub fn not_found(entity: &'static str, id: impl std::fmt::Display) -> Self {
        Self::NotFound {
            entity,
            id: id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn messages_name_the_offending_value() {
        let err = CoreError::invalid("volume", "1.5 is above 1.0");
        assert_eq!(err.to_string(), "invalid volume: 1.5 is above 1.0");

        let err = CoreError::not_found("profile", "0195c0f2-0000-7000-8000-000000000000");
        assert_eq!(
            err.to_string(),
            "profile not found: 0195c0f2-0000-7000-8000-000000000000"
        );
    }
}
