//! Equaliser settings and presets.

use super::ids::{EqPresetId, ProfileId};
use super::policies::eq_policy::{MAX_BAND_HZ, MAX_BAND_Q, MIN_BAND_HZ, MIN_BAND_Q};
use super::value_objects::{GainDb, Timestamp};
use crate::{CoreError, Result};

/// Names of the presets shipped with the application (PROJECT_MASTER 2.8).
///
/// The first was called `Flat` until migration 19. It is the preset that
/// leaves the sound alone, and "flat" is the word an engineer reaches for
/// and a listener reads as *dull*.
pub const BUILTIN_PRESET_NAMES: [&str; 9] = [
    "Standard",
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
    /// Eight-band parametric equaliser: frequency, width and gain each.
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

/// One bell of the parametric equaliser.
///
/// Three numbers, all of them the listener's to choose: where the bell sits,
/// how wide it is, and how far it lifts or cuts (`MASTER_ISSUES` 41). The
/// fields are private because all three reach a realtime filter, where a Q of
/// zero is a division by zero and a frequency past Nyquist is a filter with
/// nothing to work on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    frequency_hz: u32,
    q: f32,
    gain: GainDb,
}

impl EqBand {
    /// Validates and builds one band.
    pub fn new(frequency_hz: u32, q: f32, gain: GainDb) -> Result<Self> {
        if !(MIN_BAND_HZ..=MAX_BAND_HZ).contains(&frequency_hz) {
            return Err(CoreError::invalid(
                "eq band",
                format!("{frequency_hz} Hz is outside {MIN_BAND_HZ}..={MAX_BAND_HZ} Hz"),
            ));
        }
        if !q.is_finite() || !(MIN_BAND_Q..=MAX_BAND_Q).contains(&q) {
            return Err(CoreError::invalid(
                "eq band",
                format!("a Q of {q} is outside {MIN_BAND_Q}..={MAX_BAND_Q}"),
            ));
        }
        Ok(Self {
            frequency_hz,
            q,
            gain,
        })
    }

    /// Where the bell sits.
    pub const fn frequency_hz(self) -> u32 {
        self.frequency_hz
    }

    /// How wide it is. Higher is narrower.
    pub const fn q(self) -> f32 {
        self.q
    }

    /// How far it lifts or cuts.
    pub const fn gain(self) -> GainDb {
        self.gain
    }

    /// The same band at a different gain, which is the one thing a preset's
    /// curve changes without moving.
    pub const fn with_gain(self, gain: GainDb) -> Self {
        Self { gain, ..self }
    }
}

/// What the equaliser is set to, with no name and no identity.
///
/// A preset is one of these with a name on it. The distinction matters because
/// the listener spends most of their time between presets: they pick one, nudge
/// a control, and what is playing is no longer any preset at all.
#[derive(Debug, Clone, PartialEq)]
pub struct EqSetting {
    /// Which set of controls is in use.
    pub mode: EqMode,
    /// The three tone controls.
    pub simple: SimpleEq,
    /// The parametric bells.
    pub advanced: Vec<EqBand>,
}

impl EqSetting {
    /// Nothing lifted, nothing cut, bands at their default placement.
    pub fn flat() -> Self {
        Self {
            mode: EqMode::Simple,
            simple: SimpleEq::FLAT,
            advanced: crate::domain::policies::eq_policy::default_advanced_bands(),
        }
    }

    /// True when the equaliser would leave the signal untouched.
    pub fn is_flat(&self) -> bool {
        match self.mode {
            EqMode::Simple => self.simple.is_flat(),
            EqMode::Advanced => self.advanced.iter().all(|band| band.gain() == GainDb::ZERO),
        }
    }
}

impl From<&EqPreset> for EqSetting {
    fn from(preset: &EqPreset) -> Self {
        Self {
            mode: preset.mode,
            simple: preset.simple,
            advanced: preset.advanced.clone(),
        }
    }
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
    /// The parametric bands.
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
    use super::{BUILTIN_PRESET_NAMES, EqBand, EqMode, SimpleEq};
    use crate::domain::value_objects::GainDb;

    #[test]
    fn a_band_keeps_its_place_when_its_gain_changes() {
        let band = EqBand::new(1_000, 1.4, GainDb::ZERO).expect("in range");
        let lifted = band.with_gain(GainDb::new(6.0).expect("in range"));

        assert_eq!(lifted.frequency_hz(), 1_000);
        assert_eq!(lifted.q(), 1.4);
        assert_eq!(lifted.gain().as_db(), 6.0);
    }

    #[test]
    fn every_builtin_from_the_specification_is_present() {
        assert_eq!(BUILTIN_PRESET_NAMES.len(), 9);
        assert!(BUILTIN_PRESET_NAMES.contains(&"Standard"));
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
