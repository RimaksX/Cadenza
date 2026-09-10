//! How long a fade runs, and how loud each side of it is.
//!
//! The arithmetic of a crossfade, kept apart from the decoding it is
//! applied to. Where it is applied is `stream.rs`, on the decode thread and
//! before the ring; what it decides is here, where it can be read and tested
//! without a file, a device or a thread.
//!
//! There is no `gapless.rs` beside this one. A gapless join is the absence of
//! processing, and a file holding that would hold nothing.

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

/// How long the fade over a track of `total` frames should be.
///
/// Never more than half of it. Four seconds is a gentle overlap at the end of a
/// song and most of an interlude: on a six-second track the stored length would
/// have put two thirds of it underneath its neighbour, which is not a
/// transition but a mix. Half is the point where the fade is still a fade.
///
/// Never zero either — a fade of no length is a cut, and by the time this is
/// asked the caller has already decided a fade is what happens here.
///
/// It depends on the track's whole length rather than on what is left of it, so
/// that the answer does not change as the track plays: the same number decides
/// when the fade starts and how long it then runs, and those two disagreeing
/// would either cut the outgoing track short or leave it running into silence.
pub(crate) fn fade_length(wanted: u64, total: u64) -> u64 {
    wanted.min(total / 2).max(1)
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
    fn a_fade_takes_at_most_half_the_track_it_is_leaving() {
        let four_seconds = 44_100 * 4;

        // A song: the stored length, untouched.
        assert_eq!(fade_length(four_seconds, 44_100 * 200), four_seconds);

        // A six-second interlude: three, not four. Four would have left two
        // thirds of it playing underneath the next track.
        assert_eq!(fade_length(four_seconds, 44_100 * 6), 44_100 * 3);

        assert_eq!(
            fade_length(four_seconds, 0),
            1,
            "a fade of no length is a cut"
        );
    }
}
