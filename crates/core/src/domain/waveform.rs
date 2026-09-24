//! The shape of a track's loudness, drawn as its progress line.
//!
//! Two halves. What is measured and kept is how far below its loudest stretch
//! each stretch sits ([`levels`]); how tall that is drawn is decided when it is
//! drawn ([`heights`]). So the drawing can be tuned without measuring a single
//! file again.

/// How many stretches a track is measured in.
///
/// More than any window draws: the line takes as many as fit its width, a bar
/// every few pixels, and a wider window shows more of the same shape rather
/// than the same few bars stretched. A few hundred bytes a track.
pub const WAVEFORM_POINTS: usize = 160;

/// How finely a kept level is stored: quarter decibels, so a byte reaches
/// 63.75 dB down, far below anything drawn.
const STEPS_PER_DB: f32 = 4.0;

/// How far below the loudest stretch counts as silence, in decibels: a fade,
/// a gap between movements, the tail after the last note.
const SILENCE_DB: f32 = 40.0;

/// The narrowest range a shape is drawn across, in decibels.
///
/// A mastered track moves only a few decibels from verse to chorus, so the
/// shape is drawn across the track's own range rather than a fixed one - or
/// every bar is nearly full and the line is a fence. This stops a track that
/// barely moves at all having its last half decibel blown up into peaks.
const LEAST_RANGE_DB: f32 = 7.0;

/// How tall the quietest sounding stretch is drawn, of the full height.
const LOWEST: f32 = 0.2;

/// How much the curve opens the difference between quiet and loud out. One
/// would be a straight line; more makes the loud parts stand further clear.
const CURVE: f32 = 1.4;

/// What is kept: each stretch's loudness as quarter decibels below the
/// loudest stretch, from RMS on any scale. 255 is silence.
pub fn levels(rms: &[f32]) -> Vec<u8> {
    let loudest = rms.iter().copied().fold(0.0_f32, f32::max);
    rms.iter()
        .map(|&level| {
            if loudest <= 0.0 || level <= 0.0 {
                return u8::MAX;
            }
            let below = -20.0 * (level / loudest).log10();
            (below * STEPS_PER_DB).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// What is drawn: kept levels as heights `0..=1`.
///
/// The loudest stretch is full height and the track's quiet end - its tenth
/// quietest, so one near-silent intro does not flatten everything else - is
/// [`LOWEST`], on a curve that opens the difference between them out.
/// Silence is no bar.
pub fn heights(levels: &[u8]) -> Vec<f32> {
    let below: Vec<Option<f32>> = levels
        .iter()
        .map(|&level| Some(f32::from(level) / STEPS_PER_DB).filter(|db| *db <= SILENCE_DB))
        .collect();

    let mut sounding: Vec<f32> = below.iter().flatten().copied().collect();
    if sounding.is_empty() {
        return vec![0.0; levels.len()];
    }
    sounding.sort_by(f32::total_cmp);
    // Sorted quietest last: the tenth quietest is a tenth from the end.
    let quiet = sounding[sounding.len() - 1 - sounding.len() / 10];
    let range = quiet.max(LEAST_RANGE_DB);

    below
        .iter()
        .map(|db| match db {
            None => 0.0,
            Some(db) => {
                let along = ((range - db) / range).clamp(0.0, 1.0);
                LOWEST + (1.0 - LOWEST) * along.powf(CURVE)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{heights, levels};

    #[test]
    fn a_track_is_drawn_across_its_own_range() {
        // 0, -6 and -12 dB, and silence: a track that moves twelve decibels
        // uses the whole height for them.
        let shape = heights(&levels(&[1.0, 0.5, 0.25, 0.0]));
        assert!((shape[0] - 1.0).abs() < 1e-6, "the loudest is full height");
        assert!(
            (shape[2] - 0.2).abs() < 1e-3,
            "the quiet end is the lowest bar: {}",
            shape[2]
        );
        assert!(
            shape[1] > 0.35 && shape[1] < 0.65,
            "the middle is in the middle: {}",
            shape[1]
        );
        assert_eq!(shape[3], 0.0, "silence is no bar");
    }

    #[test]
    fn a_track_that_barely_moves_is_not_blown_up_into_peaks() {
        // Half a decibel apart: under the least range, so nearly the same.
        let shape = heights(&levels(&[1.0, 0.944]));
        assert!(shape[1] > 0.8, "{}", shape[1]);
    }

    #[test]
    fn what_is_kept_is_quarter_decibels_below_the_loudest() {
        assert_eq!(levels(&[1.0, 0.5, 0.0]), vec![0, 24, 255]);
    }

    #[test]
    fn a_silent_track_has_no_shape_rather_than_a_division_by_zero() {
        assert_eq!(heights(&levels(&[0.0, 0.0])), vec![0.0, 0.0]);
    }
}
