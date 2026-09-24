//! Musical key, and how close two keys sound to each other.

use std::fmt;

use crate::{CoreError, Result};

/// Number of pitch classes in twelve-tone equal temperament.
pub const PITCH_CLASS_COUNT: u8 = 12;

/// How much a major/minor change costs in [`MusicalKey::compatibility`].
///
/// A tuning knob, not a law of music: relative major and minor share a key
/// signature and blend well, parallel keys less so. Raise it if radio
/// transitions feel jarring, lower it if they feel monotonous.
pub const MODE_CHANGE_PENALTY: f32 = 0.1;

/// Major or minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Major scale.
    Major,
    /// Minor scale.
    Minor,
}

impl Mode {
    /// The text form stored in `track_features.mode`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Major => "major",
            Self::Minor => "minor",
        }
    }

    /// Parses the stored text form.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            other => Err(CoreError::invalid(
                "mode",
                format!("unknown mode {other:?}"),
            )),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A musical key: a pitch class (0 = C, 1 = C#, ... 11 = B) plus a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MusicalKey {
    pitch_class: u8,
    mode: Mode,
}

impl MusicalKey {
    /// Validates and wraps a key. `pitch_class` must be `0..=11`.
    pub fn new(pitch_class: u8, mode: Mode) -> Result<Self> {
        if pitch_class >= PITCH_CLASS_COUNT {
            return Err(CoreError::invalid(
                "musical key",
                format!("pitch class {pitch_class} is outside 0..=11"),
            ));
        }
        Ok(Self { pitch_class, mode })
    }

    /// The pitch class, `0..=11`.
    pub const fn pitch_class(self) -> u8 {
        self.pitch_class
    }

    /// Major or minor.
    pub const fn mode(self) -> Mode {
        self.mode
    }

    /// Position on the circle of fifths, `0..=11`.
    const fn fifths_position(self) -> u8 {
        (self.pitch_class * 7) % PITCH_CLASS_COUNT
    }

    /// Steps around the circle of fifths between two keys, `0..=6`.
    ///
    /// Adjacent keys (one step) share all but one note; six steps is the tritone,
    /// the most distant relationship.
    pub fn fifths_distance(self, other: Self) -> u8 {
        let a = i16::from(self.fifths_position());
        let b = i16::from(other.fifths_position());
        let raw = (a - b).abs();
        let wrapped = i16::from(PITCH_CLASS_COUNT) - raw;
        u8::try_from(raw.min(wrapped)).unwrap_or(0)
    }

    /// How well two keys sit next to each other, `0.0..=1.0`.
    ///
    /// Feeds `key_score` in the transition formula.
    pub fn compatibility(self, other: Self) -> f32 {
        let steps = f32::from(self.fifths_distance(other));
        let base = 1.0 - steps / 6.0;
        let penalised = if self.mode == other.mode {
            base
        } else {
            base * (1.0 - MODE_CHANGE_PENALTY)
        };
        penalised.clamp(0.0, 1.0)
    }
}

impl fmt::Display for MusicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; PITCH_CLASS_COUNT as usize] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        write!(f, "{} {}", NAMES[self.pitch_class as usize], self.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, MusicalKey};

    fn key(pitch_class: u8, mode: Mode) -> MusicalKey {
        MusicalKey::new(pitch_class, mode).expect("pitch class in range")
    }

    #[test]
    fn pitch_class_range_is_enforced() {
        assert!(MusicalKey::new(11, Mode::Major).is_ok());
        assert!(MusicalKey::new(12, Mode::Major).is_err());
    }

    #[test]
    fn distance_is_zero_for_the_same_key_and_six_for_the_tritone() {
        let c = key(0, Mode::Major);
        let g = key(7, Mode::Major);
        let f_sharp = key(6, Mode::Major);

        assert_eq!(c.fifths_distance(c), 0);
        assert_eq!(c.fifths_distance(g), 1, "C to G is one fifth");
        assert_eq!(c.fifths_distance(f_sharp), 6, "C to F# is the tritone");
    }

    #[test]
    fn distance_is_symmetric_across_the_wrap() {
        let c = key(0, Mode::Major);
        let f = key(5, Mode::Major);
        assert_eq!(c.fifths_distance(f), f.fifths_distance(c));
        assert_eq!(c.fifths_distance(f), 1, "C to F is one fourth");
    }

    #[test]
    fn compatibility_falls_off_with_distance_and_mode_change() {
        let c_major = key(0, Mode::Major);
        let g_major = key(7, Mode::Major);
        let c_minor = key(0, Mode::Minor);
        let f_sharp_major = key(6, Mode::Major);

        assert!((c_major.compatibility(c_major) - 1.0).abs() < f32::EPSILON);
        assert_eq!(c_major.compatibility(f_sharp_major), 0.0);
        assert!(c_major.compatibility(g_major) > c_major.compatibility(f_sharp_major));
        assert!(c_major.compatibility(c_minor) < c_major.compatibility(c_major));
    }

    #[test]
    fn key_names_read_the_way_a_musician_writes_them() {
        assert_eq!(key(0, Mode::Major).to_string(), "C major");
        assert_eq!(key(9, Mode::Minor).to_string(), "A minor");
    }
}
