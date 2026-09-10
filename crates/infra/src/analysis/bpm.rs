//! Tempo, from where the sound changes rather than from where it is loud.
//!
//! The method is the ordinary one and it is worth stating plainly, because The
//! fashionable alternative is forbidden here: a spectral flux onset envelope,
//! autocorrelated, with the strongest lag in the plausible range taken as the
//! beat. No model, no training data, nothing downloaded.

use super::dsp::{Spectra, squash};

/// The slowest tempo worth looking for.
pub const MIN_BPM: f32 = 60.0;

/// The fastest. Above this, what is being measured is usually half a bar.
pub const MAX_BPM: f32 = 200.0;

/// How much an envelope must repeat at the winning tempo to count as having a
/// clear beat, as the autocorrelation peak over the mean of the curve.
///
/// Measured over a real library, where it runs from 1.7 to 11.1.
const TYPICAL_PULSE: f32 = 6.0;
/// How far either side of that fills the range.
const PULSE_SPAN: f32 = 3.0;

/// How many frames the running mean of the onset envelope covers.
///
/// Subtracting a local mean is what turns "loud" into "louder than it was a
/// moment ago", which is the difference between finding the beat and finding
/// the chorus.
const SMOOTHING_FRAMES: usize = 16;

/// What the tempo estimate came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tempo {
    /// Beats per minute, absent when nothing periodic was found.
    pub bpm: Option<f32>,
    /// How much the winning lag stood out, `0.0..=1.0`.
    pub confidence: f32,
    /// How closely the thirds of the window agreed, `0.0..=1.0`.
    pub stability: f32,
    /// How pronounced the onsets themselves are, `0.0..=1.0`.
    pub beat_strength: f32,
}

impl Tempo {
    /// What a track with no discernible pulse measures.
    pub const NONE: Self = Self {
        bpm: None,
        confidence: 0.0,
        stability: 0.0,
        beat_strength: 0.0,
    };
}

/// Estimates the tempo of a window.
pub fn measure(spectra: &Spectra) -> Tempo {
    let envelope = onsets(spectra);
    let seconds = spectra.frame_seconds();
    if envelope.len() < 32 || seconds <= 0.0 {
        return Tempo::NONE;
    }

    let Some((lag, peak, mean)) = strongest_lag(&envelope, seconds) else {
        return Tempo::NONE;
    };

    let bpm = fold(60.0 / (lag * seconds));
    if mean <= 0.0 {
        return Tempo::NONE;
    }
    let confidence = squash(peak / mean, 1.6, 0.5);

    Tempo {
        bpm: Some(bpm),
        confidence,
        stability: stability(&envelope, seconds, bpm),
        // How strongly the envelope repeats at the tempo that won, which is
        // what "a clear beat" means. It used to be the tallest peak of the
        // envelope over its mean, and a single transient anywhere in ninety
        // seconds makes that ratio large: measured across the owner's library
        // it ran from 12.8 to 26.7 and every one of them squashed to about 1,
        // so danceability - which is built on it - came out between 0.87 and
        // 0.99 for the whole library and said nothing. The autocorrelation
        // ratio spreads properly on the same files: 1.7 for a piece with no
        // pulse to 11.1 for one that is all pulse.
        beat_strength: squash(peak / mean, TYPICAL_PULSE, PULSE_SPAN),
    }
}

/// How much new sound each frame brought.
///
/// Only rises count: a note stopping is not an onset, and counting it would put
/// a second peak half a beat after every real one.
fn onsets(spectra: &Spectra) -> Vec<f32> {
    let frames = spectra.frames();
    if frames.len() < 2 {
        return Vec::new();
    }

    let mut flux: Vec<f32> = frames
        .windows(2)
        .map(|pair| {
            pair[1]
                .iter()
                .zip(&pair[0])
                .map(|(now, before)| (now - before).max(0.0))
                .sum()
        })
        .collect();

    // Everything below the local average is not an onset, it is the track being
    // itself. Subtracting it leaves the events.
    let smoothed = running_mean(&flux, SMOOTHING_FRAMES);
    for (value, mean) in flux.iter_mut().zip(&smoothed) {
        *value = (*value - mean).max(0.0);
    }
    flux
}

fn running_mean(values: &[f32], span: usize) -> Vec<f32> {
    let span = span.max(1);
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let start = index.saturating_sub(span);
            let end = (index + span + 1).min(values.len());
            values[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

/// The lag at which the envelope most resembles itself.
///
/// Returns the lag in frames — interpolated, so the answer is not limited to
/// whole frames — along with the height of the peak and the average, which is
/// what makes it a confidence rather than just a winner.
fn strongest_lag(envelope: &[f32], seconds: f32) -> Option<(f32, f32, f32)> {
    let shortest = (60.0 / MAX_BPM / seconds).round().max(2.0) as usize;
    let longest = ((60.0 / MIN_BPM / seconds).round() as usize).min(envelope.len() / 2);
    if longest <= shortest {
        return None;
    }

    let scores: Vec<f32> = (shortest..=longest)
        .map(|lag| correlation(envelope, lag))
        .collect();

    // A perfectly steady beat correlates just as well with every second beat as
    // with every one, so the raw peak is a coin toss between a tempo and half of
    // it. The prior breaks the tie the way a listener does — by counting at a
    // human pace — and it is applied only to *choose*, never to the score that
    // becomes the confidence.
    let (index, _) = scores
        .iter()
        .enumerate()
        .map(|(index, score)| {
            let lag = (shortest + index) as f32;
            (index, score * prior(60.0 / (lag * seconds)))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))?;

    let peak = scores[index];
    if peak <= 0.0 {
        return None;
    }

    let mean = scores.iter().sum::<f32>() / scores.len() as f32;
    let lag = shortest as f32 + index as f32 + refine(&scores, index);
    Some((lag, peak, mean))
}

/// How likely a listener is to count at this tempo.
///
/// A bell over the logarithm of the tempo, centred where most music is counted
/// and wide enough that a genuine 70 or 170 still wins on the strength of its
/// own evidence — it needs to be about twice as strong as the alternative, which
/// a real beat is and an artefact of doubling is not.
fn prior(bpm: f32) -> f32 {
    const CENTRE_BPM: f32 = 120.0;
    const SPREAD_OCTAVES: f32 = 0.9;

    if bpm <= 0.0 {
        return 0.0;
    }
    let octaves = (bpm / CENTRE_BPM).log2() / SPREAD_OCTAVES;
    (-0.5 * octaves * octaves).exp()
}

/// Autocorrelation at one lag, normalised by the overlap so that long lags are
/// not penalised for having fewer samples to work with.
fn correlation(envelope: &[f32], lag: usize) -> f32 {
    if lag >= envelope.len() {
        return 0.0;
    }
    let overlap = envelope.len() - lag;
    let sum: f32 = envelope[lag..]
        .iter()
        .zip(&envelope[..overlap])
        .map(|(now, before)| now * before)
        .sum();
    sum / overlap as f32
}

/// Where the true peak lies between three samples of a curve.
///
/// A parabola through the winner and its neighbours. Without it the tempo can
/// only take the values whole frame-lags allow, which near 120 BPM is steps of
/// about five beats a minute — audible as wrong.
fn refine(scores: &[f32], index: usize) -> f32 {
    if index == 0 || index + 1 >= scores.len() {
        return 0.0;
    }
    let (left, middle, right) = (scores[index - 1], scores[index], scores[index + 1]);
    let denominator = left - 2.0 * middle + right;
    if denominator.abs() < f32::EPSILON {
        return 0.0;
    }
    (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
}

/// Brings a tempo into the range a listener would name.
///
/// Autocorrelation finds the period, and the period may be half a beat or two
/// beats as easily as one. Doubling and halving is the standard remedy and it
/// is honest about what it cannot know: whether a 90 BPM track is a 180 BPM
/// track counted differently is a question about the music, not the signal.
fn fold(bpm: f32) -> f32 {
    let mut bpm = bpm;
    while bpm < MIN_BPM && bpm > 0.0 {
        bpm *= 2.0;
    }
    while bpm > MAX_BPM {
        bpm /= 2.0;
    }
    bpm
}

/// How well the thirds of the window agree with the whole.
fn stability(envelope: &[f32], seconds: f32, bpm: f32) -> f32 {
    let third = envelope.len() / 3;
    if third < 32 || bpm <= 0.0 {
        return 0.0;
    }

    let mut agreement = 0.0;
    let mut counted = 0.0f32;
    for part in envelope.chunks(third).take(3) {
        if let Some((lag, _, _)) = strongest_lag(part, seconds) {
            let part_bpm = fold(60.0 / (lag * seconds));
            // A part that came out at half or double the tempo agrees about the
            // pulse and disagrees about the counting; `fold` has already made
            // them comparable, so what is left is real disagreement.
            agreement += (1.0 - (part_bpm - bpm).abs() / bpm).max(0.0);
            counted += 1.0;
        }
    }

    if counted == 0.0 {
        return 0.0;
    }
    (agreement / counted).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{MAX_BPM, MIN_BPM, Tempo, fold, measure};
    use crate::analysis::decode::Window;
    use crate::analysis::dsp::Spectra;

    /// A click track: a short burst of noise every beat, over `seconds`.
    fn clicks(bpm: f32, seconds: u32) -> Spectra {
        let rate = 22_050u32;
        let period = (60.0 / bpm * rate as f32) as usize;
        let total = (rate * seconds) as usize;

        let mut samples = vec![0.0f32; total];
        let mut state = 1u32;
        let mut position = 0;
        while position < total {
            // Twenty milliseconds of decaying noise, which is broadband enough
            // to move every bin of the spectrum at once.
            for offset in 0..(rate / 50) as usize {
                let Some(slot) = samples.get_mut(position + offset) else {
                    break;
                };
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                let decay = 1.0 - offset as f32 / (rate / 50) as f32;
                *slot = noise * decay * 0.8;
            }
            position += period;
        }

        Spectra::of(&Window { rate, samples })
    }

    #[test]
    fn a_click_track_gives_up_its_tempo() {
        for wanted in [90.0, 120.0, 140.0] {
            let tempo = measure(&clicks(wanted, 20));
            let found = tempo.bpm.expect("a tempo");

            assert!(
                (found - wanted).abs() < 3.0,
                "{wanted} BPM was measured as {found}"
            );
            assert!(
                tempo.confidence > 0.5,
                "a metronome should be convincing: {}",
                tempo.confidence
            );
            assert!(tempo.beat_strength > 0.3);
            assert!(tempo.stability > 0.8, "and it never changed");
        }
    }

    #[test]
    fn silence_has_no_tempo_to_find() {
        let quiet = Spectra::of(&Window {
            rate: 22_050,
            samples: vec![0.0; 22_050 * 5],
        });
        assert_eq!(measure(&quiet), Tempo::NONE);

        let nothing = Spectra::of(&Window {
            rate: 22_050,
            samples: Vec::new(),
        });
        assert_eq!(measure(&nothing), Tempo::NONE);
    }

    #[test]
    fn tempos_are_folded_into_the_range_a_listener_would_name() {
        assert_eq!(fold(300.0), 150.0);
        assert_eq!(fold(40.0), 80.0);
        assert!((MIN_BPM..=MAX_BPM).contains(&fold(15.0)));
        assert!((MIN_BPM..=MAX_BPM).contains(&fold(480.0)));
        assert_eq!(fold(0.0), 0.0, "and nothing stays nothing");
    }
}
