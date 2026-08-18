//! What the settings screen says about a listener.

use cadenza_core::domain::profile::Profile;
use cadenza_core::domain::value_objects::theme_mode::ThemeMode;

/// The line under a listener's name: what is theirs, in a few words.
///
/// Their two visible choices rather than a count of anything. Counting their
/// tracks would mean a query per profile on a screen that is about switching.
pub fn note(profile: &Profile) -> String {
    let theme = match profile.theme {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    };
    let history = if profile.history_enabled {
        "history kept"
    } else {
        "no history"
    };
    format!("{theme} · {history}")
}

#[cfg(test)]
mod tests {
    use super::note;
    use cadenza_core::domain::profile::{Profile, ProfileName};
    use cadenza_core::domain::value_objects::Timestamp;
    use cadenza_core::domain::value_objects::theme_mode::ThemeMode;

    fn profile(history: bool, theme: ThemeMode) -> Profile {
        let mut profile = Profile::new(
            ProfileName::new("Sasha").expect("a name"),
            Timestamp::from_millis(0),
        );
        profile.history_enabled = history;
        profile.theme = theme;
        profile
    }

    #[test]
    fn a_listener_is_described_by_what_they_chose() {
        assert_eq!(note(&profile(true, ThemeMode::Dark)), "dark · history kept");
        assert_eq!(
            note(&profile(false, ThemeMode::Light)),
            "light · no history"
        );
    }
}
