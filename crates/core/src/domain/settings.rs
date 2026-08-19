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

/// A value stored under a settings key.
///
/// Four scalar shapes and deliberately no nesting. Infrastructure encodes these
/// as JSON in `app_settings.value_json` and `profile_settings.value_json`; the
/// domain never sees the encoding, which is what keeps serialisation out of
/// `core`.
///
/// A setting that wants structure gets its own table instead. A nested blob
/// cannot be queried, indexed or constrained — the same reasoning that removed
/// `profiles.settings_json` and `profile_tracks.metadata_override_json`.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    /// A flag.
    Bool(bool),
    /// A whole number.
    Integer(i64),
    /// A fractional number. Must be finite; infinities and NaN have no JSON form.
    Float(f64),
    /// Text, including identifiers stored in their hyphenated UUID form.
    Text(String),
}

impl SettingValue {
    /// The variant name, for error messages.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::Text(_) => "text",
        }
    }

    /// Reads the value as a flag.
    ///
    /// Type mismatches are errors rather than silent defaults: a setting written
    /// as text and read as a flag means something upstream is confused, and
    /// quietly substituting `false` would hide it.
    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Self::Bool(value) => Ok(*value),
            other => Err(other.mismatch("bool")),
        }
    }

    /// Reads the value as a whole number.
    pub fn as_integer(&self) -> Result<i64> {
        match self {
            Self::Integer(value) => Ok(*value),
            other => Err(other.mismatch("integer")),
        }
    }

    /// Reads the value as a number, accepting a stored integer.
    ///
    /// JSON does not distinguish `1` from `1.0`, so a float setting that happens
    /// to hold a round number comes back as an integer. Refusing it here would
    /// make settings fail depending on their value.
    pub fn as_float(&self) -> Result<f64> {
        match self {
            Self::Float(value) => Ok(*value),
            Self::Integer(value) => Ok(*value as f64),
            other => Err(other.mismatch("float")),
        }
    }

    /// Reads the value as text.
    pub fn as_text(&self) -> Result<&str> {
        match self {
            Self::Text(value) => Ok(value),
            other => Err(other.mismatch("text")),
        }
    }

    fn mismatch(&self, wanted: &str) -> CoreError {
        CoreError::invalid(
            "setting",
            format!(
                "expected {wanted} but the stored value is {}",
                self.type_name()
            ),
        )
    }
}

impl From<bool> for SettingValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for SettingValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<String> for SettingValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SettingValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

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

/// Where the crossfade switch is kept, per profile.
///
/// Named once so that whoever writes it and whoever reads it cannot disagree
/// about a string.
pub const CROSSFADE_ENABLED_KEY: &str = "playback.crossfade_enabled";

/// Where the crossfade length is kept, in milliseconds.
pub const CROSSFADE_MS_KEY: &str = "playback.crossfade_ms";

/// Where the preloading switch is kept.
pub const PRELOAD_NEXT_KEY: &str = "playback.preload_next";

/// Where the interface scale is kept, per profile.
pub const UI_SCALE_KEY: &str = "ui.scale";

/// How large the interface is drawn, as a percentage of its design size.
///
/// PROJECT_MASTER 2.10 asks for interface scaling and, in the same section,
/// for a fixed layout — which together mean exactly this: every length grows
/// or shrinks together and nothing moves anywhere else.
///
/// Steps rather than a slider. Four sizes is a choice somebody makes once; a
/// continuous control is a thing to fiddle with, and the value between two
/// steps buys nothing an interface built on whole pixels can spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceScale(u16);

impl InterfaceScale {
    /// The sizes offered, as percentages.
    pub const STEPS: [u16; 4] = [90, 100, 110, 125];

    /// The design size, and what a profile that has never chosen gets.
    pub const DEFAULT: Self = Self(100);

    /// Validates a percentage against the steps on offer.
    pub fn new(percent: u16) -> Result<Self> {
        if Self::STEPS.contains(&percent) {
            Ok(Self(percent))
        } else {
            Err(CoreError::invalid(
                "interface scale",
                format!("{percent}% is not one of the sizes offered"),
            ))
        }
    }

    /// The percentage, for storing and for showing.
    #[must_use]
    pub const fn percent(self) -> u16 {
        self.0
    }

    /// What every length in the interface is multiplied by.
    #[must_use]
    pub fn factor(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

impl Default for InterfaceScale {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Per-profile playback preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Crossfade off, four seconds when it is turned on, and preloading on.
///
/// Written out rather than derived. A derived `Default` makes every flag false,
/// which would have shipped a `preload_next` of `false` under a doc comment
/// saying it is effectively always on — and preloading off means a gap between
/// every pair of tracks.
impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            crossfade_enabled: false,
            crossfade: CrossfadeDuration::DEFAULT,
            preload_next: true,
        }
    }
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
