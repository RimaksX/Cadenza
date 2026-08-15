//! How long a fade runs, and how loud each side of it is.
//!
//! The arithmetic of PROJECT_MASTER 8.3, kept apart from the decoding it is
//! applied to. Where it is applied is `stream.rs`, on the decode thread and
//! before the ring; what it decides is here, where it can be read and tested
//! without a file, a device or a thread.
//!
//! There is no `gapless.rs` beside this one — see `docs/MASTER_ISSUES.md` 40. A
//! gapless join is the absence of processing, and a file holding that would
//! hold nothing.

/// The gains of an equal-power crossfade at `t`, from 0 to 1.
///
/// Sine and cosine rather than a straight line: two uncorrelated signals add in
/// power, not in amplitude, so a linear fade dips by 3 dB in the middle and is
/// heard as a hole. `cos² + sin² = 1` holds the loudness flat all the way
/// across.
///
/// The first gain belongs to the track leaving, the second to the one arriving.
/// At `t = 0` the outgoing track is untouched and at `t = 1` the incoming one
/// is, which is what makes the join continuous at both ends: the fade starts at
/// exactly the level the audio was already at.
pub(crate) fn equal_power(t: f32) -> (f32, f32) {
    let angle = t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// How long the fade should actually be, in frames.
///
/// Never longer than what is left of the outgoing track: a fade that outlasts
/// the track it is fading would run out of material, finish in silence and then
/// jump. Never zero either — a fade of no length is a cut, and the caller has
/// already decided that a fade is what happens here.
pub(crate) fn fade_length(wanted: u64, remaining: u64) -> u64 {
    wanted.min(remaining).max(1)
}

#[cfg(test)]
mod tests {
    use super::{equal_power, fade_length};

    #[test]
    fn the_two_gains_hold_the_loudness_flat() {
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let (out, into) = equal_power(t);
            assert!(
                (out * out + into * into - 1.0).abs() < 1e-5,
                "power dips at {t}"
            );
        }
    }

    #[test]
    fn each_end_of_the_fade_is_the_untouched_track() {
        let (out, into) = equal_power(0.0);
        assert!((out - 1.0).abs() < 1e-6 && into.abs() < 1e-6);

        let (out, into) = equal_power(1.0);
        assert!(out.abs() < 1e-6 && (into - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_position_outside_the_fade_is_clamped_rather_than_extrapolated() {
        assert_eq!(equal_power(-1.0), equal_power(0.0));
        assert_eq!(equal_power(2.0), equal_power(1.0));
    }

    #[test]
    fn a_fade_is_never_longer_than_what_is_left_to_fade() {
        assert_eq!(fade_length(176_400, 88_200), 88_200);
        assert_eq!(fade_length(88_200, 176_400), 88_200);
        assert_eq!(fade_length(88_200, 0), 1, "a fade of no length is a cut");
    }
}
