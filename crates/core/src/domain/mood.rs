//! Mood and activity presets that steer radio selection.

use super::ids::{MoodId, ProfileId};
use super::playback::TransitionProfile;
use super::value_objects::Timestamp;

/// The moods shipped with the application (PROJECT_MASTER 2.7).
pub const BUILTIN_MOOD_NAMES: [&str; 8] = [
    "Workout", "Focus", "Chill", "Party", "Driving", "Sleep", "Gaming", "Morning",
];

/// A mood or activity the listener can start radio from.
///
/// The rule and weight payloads stay serialised at this stage. Their shape is
/// settled in M13 alongside the ranking formula that consumes them; modelling
/// them now would be inventing a schema for an algorithm that does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoodPreset {
    /// Stable identifier.
    pub id: MoodId,
    /// Owning profile, or `None` for a built-in mood shared by everyone.
    pub profile_id: Option<ProfileId>,
    /// Display name.
    pub name: String,
    /// True for moods shipped with the application.
    pub is_builtin: bool,
    /// Serialised feature constraints, e.g. acceptable BPM and energy ranges.
    pub feature_rules_json: Option<String>,
    /// Serialised per-genre preference multipliers.
    pub genre_boost_json: Option<String>,
    /// Serialised ranking weights.
    pub ranking_weights_json: Option<String>,
    /// How tracks in this mood should hand over to each other.
    pub transition: TransitionProfile,
    /// When the preset was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

impl MoodPreset {
    /// True when the listener may edit or delete this mood.
    ///
    /// Custom moods are allowed (PROJECT_MASTER 2.7); built-ins are not editable,
    /// so that a listener can always get back to a known-good starting point.
    pub const fn is_editable(&self) -> bool {
        !self.is_builtin
    }
}

#[cfg(test)]
mod tests {
    use super::BUILTIN_MOOD_NAMES;

    #[test]
    fn every_builtin_from_the_specification_is_present() {
        assert_eq!(BUILTIN_MOOD_NAMES.len(), 8);
        for expected in [
            "Workout", "Focus", "Chill", "Party", "Driving", "Sleep", "Gaming", "Morning",
        ] {
            assert!(
                BUILTIN_MOOD_NAMES.contains(&expected),
                "{expected} is missing"
            );
        }
    }
}
