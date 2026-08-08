//! Dark or light appearance.

use std::fmt;

use crate::{CoreError, Result};

/// The appearance a profile has selected.
///
/// Only two variants: PROJECT_MASTER 2.10 requires a dark and a light theme and a
/// fixed, non-user-rearrangeable layout. Following the operating system setting is
/// not currently a requirement, so it is not an option here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    /// Dark appearance.
    #[default]
    Dark,
    /// Light appearance.
    Light,
}

impl ThemeMode {
    /// The text form stored in `profiles.theme`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            other => Err(CoreError::invalid(
                "theme",
                format!("unknown theme {other:?}"),
            )),
        }
    }
}

impl fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeMode;

    #[test]
    fn text_form_round_trips() {
        for theme in [ThemeMode::Dark, ThemeMode::Light] {
            assert_eq!(ThemeMode::parse(theme.as_str()).expect("round trip"), theme);
        }
    }

    #[test]
    fn unknown_themes_are_rejected_rather_than_defaulted() {
        assert!(ThemeMode::parse("solarized").is_err());
    }
}
