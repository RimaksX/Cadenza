//! Library folders and playback settings.

use std::path::PathBuf;

use super::ids::{ProfileFolderId, ProfileId};
use super::value_objects::{DurationMs, Timestamp};
use crate::{CoreError, Result};

/// Shortest crossfade allowed (PROJECT_MASTER 2.4).
pub const MIN_CROSSFADE: DurationMs = DurationMs::from_secs(3);

/// Longest crossfade allowed.
pub const MAX_CROSSFADE: DurationMs = DurationMs::from_secs(5);

/// Crossfade length used unless the listener changes it.
pub const DEFAULT_CROSSFADE: DurationMs = DurationMs::from_secs(4);

/// A validated crossfade length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CrossfadeDuration(DurationMs);

impl CrossfadeDuration {
    /// The four-second default.
    pub const DEFAULT: Self = Self(DEFAULT_CROSSFADE);

    /// Validates a crossfade length against the allowed range.
    pub fn new(duration: DurationMs) -> Result<Self> {
        if duration < MIN_CROSSFADE || duration > MAX_CROSSFADE {
            return Err(CoreError::invalid(
                "crossfade",
                format!(
                    "{} ms is outside {}..={} ms",
                    duration.as_millis(),
                    MIN_CROSSFADE.as_millis(),
                    MAX_CROSSFADE.as_millis()
                ),
            ));
        }
        Ok(Self(duration))
    }

    /// The length as a span.
    pub const fn as_duration(self) -> DurationMs {
        self.0
    }
}

impl Default for CrossfadeDuration {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A folder a profile has added to its library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFolder {
    /// Stable identifier.
    pub id: ProfileFolderId,
    /// Owning profile. Two profiles may watch the same folder independently.
    pub profile_id: ProfileId,
    /// Root path to scan.
    pub path: PathBuf,
    /// Whether to descend into subdirectories.
    pub include_subfolders: bool,
    /// Whether the folder is currently scanned and watched.
    pub enabled: bool,
    /// When the folder was last fully scanned.
    pub last_scan_at: Option<Timestamp>,
}

/// Per-profile playback preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaybackSettings {
    /// Whether ordinary track changes crossfade.
    ///
    /// Radio and playlists stay gapless regardless; this switch governs the
    /// transitions the crossfade rule applies to (PROJECT_MASTER 2.4).
    pub crossfade_enabled: bool,
    /// Crossfade length, when enabled.
    pub crossfade: CrossfadeDuration,
    /// Whether the next track is decoded ahead of time.
    ///
    /// Effectively always on; exposed so it can be turned off when diagnosing
    /// audio problems.
    pub preload_next: bool,
}

#[cfg(test)]
mod tests {
    use super::{CrossfadeDuration, DEFAULT_CROSSFADE, MAX_CROSSFADE, MIN_CROSSFADE};
    use crate::domain::value_objects::DurationMs;

    #[test]
    fn the_documented_range_is_accepted_and_nothing_else() {
        assert!(CrossfadeDuration::new(MIN_CROSSFADE).is_ok());
        assert!(CrossfadeDuration::new(MAX_CROSSFADE).is_ok());
        assert!(CrossfadeDuration::new(DEFAULT_CROSSFADE).is_ok());
        assert!(CrossfadeDuration::new(DurationMs::from_secs(2)).is_err());
        assert!(CrossfadeDuration::new(DurationMs::from_secs(6)).is_err());
    }

    #[test]
    fn the_default_is_four_seconds() {
        assert_eq!(
            CrossfadeDuration::default().as_duration(),
            DurationMs::from_secs(4)
        );
    }
}
