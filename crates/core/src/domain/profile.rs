//! Listener profile.

use std::fmt;

use super::ids::ProfileId;
use super::value_objects::{ThemeMode, Timestamp};
use crate::{CoreError, Result};

/// Longest profile name accepted, in characters.
pub const MAX_PROFILE_NAME_CHARS: usize = 64;

/// How long listening history is kept when it is enabled.
///
/// PROJECT_MASTER 1.4 and 2.6 both state 30 days, and rule 12.1 repeats it. The
/// schema exposes the value as a column so it can be read back and shown to the
/// user, but it is a ceiling, not a free setting: a profile may keep history for
/// fewer days, never more. Raising this is a privacy-policy change and requires
/// amending the master file.
pub const HISTORY_RETENTION_DAYS: u16 = 30;

/// A validated profile name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileName(String);

impl ProfileName {
    /// Trims surrounding whitespace and validates the result.
    pub fn new(raw: impl Into<String>) -> Result<Self> {
        let trimmed = raw.into().trim().to_owned();
        if trimmed.is_empty() {
            return Err(CoreError::invalid("profile name", "must not be empty"));
        }
        let length = trimmed.chars().count();
        if length > MAX_PROFILE_NAME_CHARS {
            return Err(CoreError::invalid(
                "profile name",
                format!("{length} characters exceeds the {MAX_PROFILE_NAME_CHARS} character limit"),
            ));
        }
        Ok(Self(trimmed))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A listener, owning their own library, playlists, queue, history and settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Stable identifier. Every row of user data carries this.
    pub id: ProfileId,
    /// Display name.
    pub name: ProfileName,
    /// When the profile was created.
    pub created_at: Timestamp,
    /// Whether listening events are recorded at all.
    ///
    /// Asked once during profile setup. When off, nothing is written — not
    /// reduced detail, not anonymised rows, nothing (PROJECT_MASTER 2.6).
    pub history_enabled: bool,
    /// How many days of history to keep, capped at [`HISTORY_RETENTION_DAYS`].
    pub history_retention_days: u16,
    /// Selected appearance.
    pub theme: ThemeMode,
}

impl Profile {
    /// Creates a profile with history disabled, which is the safe default until
    /// the setup wizard has asked.
    pub fn new(name: ProfileName, created_at: Timestamp) -> Self {
        Self {
            id: ProfileId::new(),
            name,
            created_at,
            history_enabled: false,
            history_retention_days: HISTORY_RETENTION_DAYS,
            theme: ThemeMode::default(),
        }
    }

    /// The retention window actually in force, never exceeding the policy cap.
    pub fn effective_retention_days(&self) -> u16 {
        self.history_retention_days.min(HISTORY_RETENTION_DAYS)
    }
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_RETENTION_DAYS, MAX_PROFILE_NAME_CHARS, Profile, ProfileName};
    use crate::domain::value_objects::Timestamp;

    fn name(text: &str) -> ProfileName {
        ProfileName::new(text).expect("valid name")
    }

    #[test]
    fn names_are_trimmed_and_must_not_be_empty() {
        assert_eq!(name("  Sasha  ").as_str(), "Sasha");
        assert!(ProfileName::new("").is_err());
        assert!(ProfileName::new("   ").is_err());
    }

    #[test]
    fn overlong_names_are_rejected_by_character_count_not_bytes() {
        let cyrillic = "я".repeat(MAX_PROFILE_NAME_CHARS);
        assert!(
            ProfileName::new(cyrillic).is_ok(),
            "64 multi-byte characters is 64 characters, not 128"
        );
        assert!(ProfileName::new("x".repeat(MAX_PROFILE_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn history_is_off_until_the_wizard_asks() {
        let profile = Profile::new(name("Sasha"), Timestamp::from_millis(1_754_611_200_000));
        assert!(!profile.history_enabled);
    }

    #[test]
    fn retention_never_exceeds_the_policy_cap() {
        let mut profile = Profile::new(name("Sasha"), Timestamp::UNIX_EPOCH);
        profile.history_retention_days = 365;
        assert_eq!(profile.effective_retention_days(), HISTORY_RETENTION_DAYS);

        profile.history_retention_days = 7;
        assert_eq!(profile.effective_retention_days(), 7);
    }
}
