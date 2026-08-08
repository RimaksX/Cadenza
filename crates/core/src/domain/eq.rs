//! Equaliser settings and presets.

use super::ids::{EqPresetId, ProfileId};
use super::value_objects::{GainDb, Timestamp};
use crate::{CoreError, Result};

/// Names of the presets shipped with the application (PROJECT_MASTER 2.8).
pub const BUILTIN_PRESET_NAMES: [&str; 9] = [
    "Flat",
    "Pop",
    "Rock",
    "Classical",
    "Electronic",
    "Vocal Enhance",
    "Spatial Enhance",
    "Bass Boost",
    "Treble Boost",
];

/// Which set of controls is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EqMode {
    /// Three controls: bass, mid, treble.
    #[default]
    Simple,
    /// Ten-band graphic equaliser.
    Advanced,
}

impl EqMode {
    /// The text form stored in `eq_presets.mode`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Advanced => "advanced",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "simple" => Ok(Self::Simple),
            "advanced" => Ok(Self::Advanced),
            other => Err(CoreError::invalid(
                "eq mode",
                format!("unknown mode {other:?}"),
            )),
        }
    }
}

/// The three-control equaliser.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SimpleEq {
    /// Low shelf gain.
    pub bass: GainDb,
    /// Midrange peaking gain.
    pub mid: GainDb,
    /// High shelf gain.
    pub treble: GainDb,
}

impl SimpleEq {
    /// No adjustment on any control.
    pub const FLAT: Self = Self {
        bass: GainDb::ZERO,
        mid: GainDb::ZERO,
        treble: GainDb::ZERO,
    };

    /// True when the equaliser would leave the signal untouched.
    pub fn is_flat(&self) -> bool {
        self.bass == GainDb::ZERO && self.mid == GainDb::ZERO && self.treble == GainDb::ZERO
    }
}

/// One band of the graphic equaliser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    /// Centre frequency in hertz.
    pub frequency_hz: u32,
    /// Gain applied at that frequency.
    pub gain: GainDb,
}

/// A named, storable equaliser configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct EqPreset {
    /// Stable identifier.
    pub id: EqPresetId,
    /// Owning profile, or `None` for a built-in preset shared by everyone.
    ///
    /// Built-ins are not user data, which is why they are allowed to have no
    /// `profile_id` without violating the isolation rule.
    pub profile_id: Option<ProfileId>,
    /// Display name.
    pub name: String,
    /// True for presets shipped with the application, which cannot be edited.
    pub is_builtin: bool,
    /// Which control set the preset applies.
    pub mode: EqMode,
    /// Three-control values.
    pub simple: SimpleEq,
    /// Ten-band values.
    pub advanced: Vec<EqBand>,
    /// When the preset was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

impl EqPreset {
    /// True when the preset may be renamed, edited or deleted.
    pub const fn is_editable(&self) -> bool {
        !self.is_builtin
    }
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_PRESET_NAMES, EqMode, SimpleEq};
    use crate::domain::value_objects::GainDb;

    #[test]
    fn every_builtin_from_the_specification_is_present() {
        assert_eq!(BUILTIN_PRESET_NAMES.len(), 9);
        assert!(BUILTIN_PRESET_NAMES.contains(&"Flat"));
        assert!(BUILTIN_PRESET_NAMES.contains(&"Spatial Enhance"));
    }

    #[test]
    fn flat_means_untouched() {
        assert!(SimpleEq::FLAT.is_flat());
        assert!(SimpleEq::default().is_flat());

        let boosted = SimpleEq {
            bass: GainDb::new(3.0).expect("in range"),
            ..SimpleEq::FLAT
        };
        assert!(!boosted.is_flat());
    }

    #[test]
    fn mode_text_form_round_trips() {
        for mode in [EqMode::Simple, EqMode::Advanced] {
            assert_eq!(EqMode::parse(mode.as_str()).expect("round trip"), mode);
        }
    }
}
