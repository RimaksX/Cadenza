//! Library folders and playback settings.

use std::path::PathBuf;

use super::ids::{ProfileFolderId, ProfileId};
use super::value_objects::{DurationMs, Timestamp};
use crate::{CoreError, Result};

/// Shortest crossfade allowed.
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

/// How fast music plays, as a percentage of its own speed.
///
/// Faster and slower together with pitch, the way a record at the wrong speed
/// sounds: "slowed" is the lower, darker version of a song, not the same song
/// stretched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlaybackSpeed(u16);

impl PlaybackSpeed {
    /// The music as it was recorded.
    pub const NORMAL: Self = Self(100);

    /// The slowest: where "slowed" versions usually sit.
    pub const MIN_PERCENT: u16 = 80;

    /// The fastest.
    pub const MAX_PERCENT: u16 = 120;

    /// The step the line moves in.
    pub const STEP_PERCENT: u16 = 2;

    /// Validates a percentage against the range and the step.
    pub fn new(percent: u16) -> Result<Self> {
        if !(Self::MIN_PERCENT..=Self::MAX_PERCENT).contains(&percent)
            || !percent.is_multiple_of(Self::STEP_PERCENT)
        {
            return Err(CoreError::invalid(
                "speed",
                format!("{percent}% is not a step of 2 within 80..=120%"),
            ));
        }
        Ok(Self(percent))
    }

    /// The percentage.
    pub const fn percent(self) -> u16 {
        self.0
    }

    /// The factor time is multiplied by: 0.8 is slower.
    pub fn factor(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self::NORMAL
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

/// Where the loudness normalisation switch is kept, per profile.
pub const NORMALISE_KEY: &str = "playback.normalise";

/// Where the playback speed is kept, per profile, as a percentage.
pub const SPEED_KEY: &str = "playback.speed_percent";

/// When the favourites were last cleared, per profile, in unix milliseconds.
///
/// Plays before it no longer count towards the list: a cleared list that
/// refilled itself from last week's listening would not have been cleared.
pub const FAVOURITES_CLEARED_KEY: &str = "favourites.cleared_at";

/// Where the preloading switch is kept.
pub const PRELOAD_NEXT_KEY: &str = "playback.preload_next";

/// Where the output level is kept, per profile.
///
/// A scalar in `0.0..=1.0`, the same number the slider holds. Kept per profile
/// because it is a preference and not a property of the machine: two listeners
/// sharing a computer do not share what "loud enough" means.
pub const VOLUME_KEY: &str = "playback.volume";

/// Where the interface scale is kept, per profile.
pub const UI_SCALE_KEY: &str = "ui.scale";

/// Where the interface language is kept, per profile.
pub const UI_LANGUAGE_KEY: &str = "ui.language";

/// Where the window was and how large, as it was left.
///
/// Per machine rather than per profile: it describes this computer's screens,
/// which two listeners share. Physical pixels and the frame's outer corner, as
/// the platform reports and takes them. Kept as five scalars, not one value
/// with structure - see [`SettingValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Maximised when it was closed. The size and place are then the ones it
    /// had before, which is where it goes back to when un-maximised.
    pub maximized: bool,
}

impl WindowPlacement {
    pub const X_KEY: &'static str = "ui.window_x";
    pub const Y_KEY: &'static str = "ui.window_y";
    pub const WIDTH_KEY: &'static str = "ui.window_width";
    pub const HEIGHT_KEY: &'static str = "ui.window_height";
    pub const MAXIMIZED_KEY: &'static str = "ui.window_maximized";
}

/// A column at the side of the window that folds away to a strip.
///
/// Per profile, as the scale is: how much room a listener gives the page is
/// theirs, not the machine's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideColumn {
    /// The destinations down the left.
    Navigation,
    /// What is playing and what comes next, down the right.
    NowPlaying,
}

impl SideColumn {
    /// Where whether it is folded is kept.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Navigation => "ui.navigation_folded",
            Self::NowPlaying => "ui.now_playing_folded",
        }
    }

    /// Where whether the now-playing panel folds to its strip is kept. Off,
    /// it folds away altogether and the player bar carries the way back.
    pub const NOW_PLAYING_RAIL_KEY: &'static str = "ui.now_playing_rail";
}

/// What language the interface speaks.
///
/// Per profile rather than per machine, for the same reason the theme is: two
/// people sharing a computer do not share a first language.
///
/// Stored as the tag the markup's translation bundle is keyed by, so what is
/// written here is what `select_bundled_translation` is handed. An empty tag is
/// the language the interface was written in, which is why [`Self::English`]
/// carries one - there is no English catalogue, only the original strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// The language the interface is written in.
    #[default]
    English,
    /// Russian.
    Russian,
}

impl Language {
    /// Every language on offer, in the order the settings screen lists them.
    pub const ALL: [Self; 2] = [Self::English, Self::Russian];

    /// The tag stored in settings and handed to the translation bundle.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "",
            Self::Russian => "ru",
        }
    }

    /// What this language calls itself.
    ///
    /// Never translated: a listener looking for their own language is looking
    /// for the word they would write, not for its name in a language they do
    /// not read.
    #[must_use]
    pub const fn endonym(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }

    /// Reads a stored tag, treating anything unknown as the original.
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "ru" => Self::Russian,
            _ => Self::English,
        }
    }
}

/// How large the interface is drawn, as a percentage of its design size.
///
/// The interface scales, and in the same breath for a fixed layout — which
/// together mean exactly this: every length grows or shrinks together and
/// nothing moves anywhere else.
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
    /// transitions the crossfade rule applies to.
    pub crossfade_enabled: bool,
    /// Crossfade length, when enabled.
    pub crossfade: CrossfadeDuration,
    /// Whether the next track is decoded ahead of time.
    ///
    /// Effectively always on; exposed so it can be turned off when diagnosing
    /// audio problems.
    pub preload_next: bool,
    /// Whether loud tracks are turned down to the level of the rest.
    pub normalise: bool,
    /// How fast music plays.
    pub speed: PlaybackSpeed,
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
            normalise: false,
            speed: PlaybackSpeed::NORMAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CrossfadeDuration, DEFAULT_CROSSFADE, MAX_CROSSFADE, MIN_CROSSFADE, PlaybackSpeed,
    };

    #[test]
    fn a_speed_is_a_step_of_the_line() {
        for percent in (80..=120).step_by(2) {
            assert!(PlaybackSpeed::new(percent).is_ok(), "{percent}");
        }
        assert!(PlaybackSpeed::new(78).is_err());
        assert!(PlaybackSpeed::new(122).is_err());
        assert!(PlaybackSpeed::new(85).is_err(), "between two steps");
        assert!((PlaybackSpeed::new(86).expect("in range").factor() - 0.86).abs() < 1e-9);
    }
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
