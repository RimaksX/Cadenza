//! What the radio screen shows about a mood and a station.

use cadenza_core::domain::mood::{FeatureBand, MoodPreset, MoodRules};

/// What a mood is asking the library for, in a few words.
///
/// The bands themselves rather than a description of them: a listener choosing
/// between Focus and Chill is choosing between two claims about tempo and
/// loudness, and the claims are short enough to simply show.
pub fn asks_for(rules: &MoodRules) -> String {
    let mut parts = Vec::new();

    if let Some(band) = rules.bpm {
        parts.push(format!("{:.0}-{:.0} bpm", band.low, band.high));
    }
    if let Some(band) = rules.energy {
        parts.push(format!("energy {}", strength(band)));
    }
    if let Some(band) = rules.valence {
        parts.push(format!("mood {}", strength(band)));
    }
    if let Some(band) = rules.danceability {
        parts.push(format!("pull {}", strength(band)));
    }

    if parts.is_empty() {
        return "anything at all".to_owned();
    }
    parts.join(" · ")
}

/// Where a normalised band sits, in a word.
fn strength(band: FeatureBand) -> &'static str {
    let middle = (band.low + band.high) / 2.0;
    if middle < 0.35 {
        "low"
    } else if middle < 0.65 {
        "middling"
    } else {
        "high"
    }
}

/// The line under the title: what is playing, or what to do about it.
pub fn summary_line(playing: Option<&MoodPreset>, waiting: usize) -> String {
    match playing {
        None => "pick a mood — it plays from what you already have".to_owned(),
        Some(mood) => {
            let noun = if waiting == 1 { "track" } else { "tracks" };
            format!("{} · {waiting} {noun} lined up", mood.name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{asks_for, summary_line};
    use cadenza_core::domain::ids::MoodId;
    use cadenza_core::domain::mood::{FeatureBand, MoodPreset, MoodRules};
    use cadenza_core::domain::playback::TransitionProfile;
    use cadenza_core::domain::value_objects::Timestamp;

    fn mood(name: &str) -> MoodPreset {
        MoodPreset {
            id: MoodId::new(),
            profile_id: None,
            name: name.to_owned(),
            is_builtin: true,
            rules: MoodRules::default(),
            genre_boost_json: None,
            ranking_weights_json: None,
            transition: TransitionProfile::Gapless,
            created_at: Timestamp::from_millis(0),
            updated_at: Timestamp::from_millis(0),
        }
    }

    #[test]
    fn a_mood_says_what_it_asks_for() {
        let rules = MoodRules {
            bpm: Some(FeatureBand::new(125.0, 175.0, 25.0)),
            energy: Some(FeatureBand::new(0.65, 1.0, 0.25)),
            ..MoodRules::default()
        };
        assert_eq!(asks_for(&rules), "125-175 bpm · energy high");
    }

    #[test]
    fn a_mood_that_asks_for_nothing_says_that_too() {
        assert_eq!(asks_for(&MoodRules::default()), "anything at all");
    }

    #[test]
    fn the_summary_says_what_is_playing_or_what_to_do() {
        assert_eq!(
            summary_line(None, 0),
            "pick a mood — it plays from what you already have"
        );
        assert_eq!(
            summary_line(Some(&mood("Workout")), 8),
            "Workout · 8 tracks lined up"
        );
        assert_eq!(
            summary_line(Some(&mood("Sleep")), 1),
            "Sleep · 1 track lined up"
        );
    }
}
