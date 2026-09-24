//! Where playback currently sits inside a track.

use std::fmt;

use super::duration::DurationMs;

/// Elapsed position within the current track.
///
/// A distinct type from [`DurationMs`] so that "how far in we are" and "how long
/// the track is" cannot be swapped by accident — the two are mixed constantly by
/// seek, progress display and the previous-track rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PlaybackPosition(DurationMs);

impl PlaybackPosition {
    /// The start of the track.
    pub const START: Self = Self(DurationMs::ZERO);

    /// Wraps a millisecond offset.
    pub const fn from_millis(millis: u64) -> Self {
        Self(DurationMs::from_millis(millis))
    }

    /// Wraps a second offset.
    pub const fn from_secs(secs: u64) -> Self {
        Self(DurationMs::from_secs(secs))
    }

    /// The offset in milliseconds.
    pub const fn as_millis(self) -> u64 {
        self.0.as_millis()
    }

    /// The offset as a span from the start of the track.
    pub const fn elapsed(self) -> DurationMs {
        self.0
    }

    /// How far through a track of length `total` this position is, `0.0..=1.0`.
    pub fn fraction_of(self, total: DurationMs) -> f32 {
        self.0.fraction_of(total)
    }

    /// Moves forward, saturating at [`u64::MAX`].
    pub const fn saturating_add(self, span: DurationMs) -> Self {
        Self(self.0.saturating_add(span))
    }

    /// Moves backward, saturating at the start of the track.
    pub const fn saturating_sub(self, span: DurationMs) -> Self {
        Self(self.0.saturating_sub(span))
    }
}

impl fmt::Display for PlaybackPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationMs, PlaybackPosition};

    #[test]
    fn seeking_backwards_stops_at_the_start() {
        let position = PlaybackPosition::from_secs(5);
        assert_eq!(
            position.saturating_sub(DurationMs::from_secs(30)),
            PlaybackPosition::START
        );
    }

    #[test]
    fn progress_is_reported_against_the_track_length() {
        let halfway = PlaybackPosition::from_secs(100);
        let total = DurationMs::from_secs(200);
        assert!((halfway.fraction_of(total) - 0.5).abs() < f32::EPSILON);
    }
}
