//! Mood and activity presets that steer radio selection.

use super::ids::{MoodId, ProfileId};
use super::playback::TransitionProfile;
use super::value_objects::Timestamp;

/// The moods shipped with the application.
pub const BUILTIN_MOOD_NAMES: [&str; 8] = [
    "Workout", "Focus", "Chill", "Party", "Driving", "Sleep", "Gaming", "Morning",
];

/// A band a feature is asked to fall inside.
///
/// Soft-edged rather than a filter: a track four beats a minute outside the
/// band for Workout is not the wrong track, it is very slightly less right. A
/// hard cut-off would make a mood in a small library produce nothing at all,
/// which is the one outcome radio may not have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeatureBand {
    /// Bottom of the band.
    pub low: f32,
    /// Top of it.
    pub high: f32,
    /// How far outside the band a value can be before it scores nothing.
    ///
    /// In the same units as the band, so a tempo band's falloff is in beats per
    /// minute and a normalised feature's is in hundredths of its range.
    pub falloff: f32,
}

impl FeatureBand {
    /// Builds a band, ordering its ends if they arrive the wrong way round.
    pub fn new(low: f32, high: f32, falloff: f32) -> Self {
        Self {
            low: low.min(high),
            high: low.max(high),
            falloff: falloff.max(f32::EPSILON),
        }
    }

    /// How well a value sits in the band, `0.0..=1.0`.
    pub fn fits(&self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        let outside = if value < self.low {
            self.low - value
        } else if value > self.high {
            value - self.high
        } else {
            return 1.0;
        };
        (1.0 - outside / self.falloff).clamp(0.0, 1.0)
    }
}

/// What a mood asks of the music.
///
/// Every field is optional because a mood is a statement about the few things
/// that matter to it. Focus cares that a track is quiet and says nothing about
/// whether it is happy; asking every mood to have an opinion about every
/// feature would mean inventing opinions.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MoodRules {
    /// Preferred tempo, in beats per minute.
    pub bpm: Option<FeatureBand>,
    /// Preferred intensity, `0.0..=1.0`.
    pub energy: Option<FeatureBand>,
    /// Preferred musical positivity, `0.0..=1.0`.
    pub valence: Option<FeatureBand>,
    /// Preferred rhythmic pull, `0.0..=1.0`.
    pub danceability: Option<FeatureBand>,
}

impl MoodRules {
    /// True when the mood asks for nothing at all.
    ///
    /// Such a mood fits everything equally, which is a legitimate thing for a
    /// listener to build: "anything, but transitions that flow".
    pub const fn is_unconstrained(&self) -> bool {
        self.bpm.is_none()
            && self.energy.is_none()
            && self.valence.is_none()
            && self.danceability.is_none()
    }
}

/// A mood or activity the listener can start radio from.
#[derive(Debug, Clone, PartialEq)]
pub struct MoodPreset {
    /// Stable identifier.
    pub id: MoodId,
    /// Owning profile, or `None` for a built-in mood shared by everyone.
    pub profile_id: Option<ProfileId>,
    /// Display name.
    pub name: String,
    /// True for moods shipped with the application.
    pub is_builtin: bool,
    /// What the mood asks of the music.
    ///
    /// Typed rather than the JSON the column holds: the domain has no business
    /// parsing an encoding, and the adapter that reads the row is where the
    /// encoding belongs.
    pub rules: MoodRules,
    /// Serialised per-genre preference multipliers.
    ///
    /// Still opaque. Genre affinity is a ranking term and needs a listener's
    /// own genres in front of it before the shape can be chosen; radio works
    /// without it, and inventing a schema for an unwritten scorer is how
    /// schemas become wrong.
    pub genre_boost_json: Option<String>,
    /// Serialised ranking weights.
    ///
    /// Also opaque, and for now always absent: every mood ranks with
    /// [`crate::domain::policies::radio_policy::RankingWeights::DEFAULT`]. A
    /// mood that wants its own is a per-mood override, and nothing has asked
    /// for one yet.
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
    /// Custom moods are allowed; built-ins are not editable, so that a listener
    /// can always get back to a known-good starting point.
    pub const fn is_editable(&self) -> bool {
        !self.is_builtin
    }
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_MOOD_NAMES, FeatureBand, MoodRules};

    #[test]
    fn a_value_inside_the_band_fits_perfectly() {
        let band = FeatureBand::new(120.0, 160.0, 20.0);
        assert_eq!(band.fits(120.0), 1.0);
        assert_eq!(band.fits(140.0), 1.0);
        assert_eq!(band.fits(160.0), 1.0);
    }

    #[test]
    fn just_outside_is_nearly_right_and_far_outside_is_nothing() {
        let band = FeatureBand::new(120.0, 160.0, 20.0);

        assert!(
            (band.fits(110.0) - 0.5).abs() < 1e-6,
            "half a falloff below"
        );
        assert!(
            (band.fits(170.0) - 0.5).abs() < 1e-6,
            "half a falloff above"
        );
        assert_eq!(band.fits(60.0), 0.0);
        assert_eq!(band.fits(240.0), 0.0);
    }

    #[test]
    fn a_band_given_backwards_still_means_what_was_meant() {
        let band = FeatureBand::new(160.0, 120.0, 20.0);
        assert_eq!(band.fits(140.0), 1.0);
    }

    #[test]
    fn nonsense_never_fits_and_never_divides_by_zero() {
        assert_eq!(FeatureBand::new(0.0, 1.0, 0.1).fits(f32::NAN), 0.0);

        let flat = FeatureBand::new(0.5, 0.5, 0.0);
        assert_eq!(flat.fits(0.5), 1.0);
        assert_eq!(flat.fits(0.6), 0.0);
    }

    #[test]
    fn a_mood_that_asks_for_nothing_says_so() {
        assert!(MoodRules::default().is_unconstrained());
        assert!(
            !MoodRules {
                energy: Some(FeatureBand::new(0.0, 0.3, 0.2)),
                ..MoodRules::default()
            }
            .is_unconstrained()
        );
    }

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
