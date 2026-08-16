//! Turning what was played into something to draw.
//!
//! The last stage of PROJECT_MASTER 8.1, and the only one that produces no
//! sound. The callback's part of it is a copy — section 8.2 allows passing
//! frames to a tap and nothing more — so everything here runs on the thread
//! that asks, which is the interface's, at the rate section 2.9 caps: thirty
//! times a second and not once while nobody is looking.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::stream::Shared;

/// How many samples one reading looks at.
///
/// A thousand at 48 kHz is twenty-one milliseconds — long enough to resolve a
/// bass note, short enough that a drum still arrives as a hit rather than as a
/// smear. It is also a power of two, which is the transform's happiest length.
const WINDOW: usize = 1_024;

/// The lowest frequency the bars cover.
///
/// Below this is felt rather than heard, and a bar for it would spend most of
/// its life at rest.
const LOWEST_HZ: f32 = 40.0;

/// The highest.
const HIGHEST_HZ: f32 = 16_000.0;

/// How quiet a band has to be to leave its bar on the floor.
///
/// Sixty decibels below full scale. Anything under this is either silence or a
/// noise floor, and drawing it makes a row of bars that never quite rests.
const FLOOR_DB: f32 = -60.0;

/// How fast a bar rises towards a louder reading, per frame at thirty a second.
///
/// Immediate: a transient that arrives late reads as a different instrument.
const ATTACK: f32 = 0.6;

/// How fast it falls back.
///
/// Slower than it rises, which is what makes a row of bars look like sound
/// rather than like a flicker. A bar left to drop as fast as it climbs spends
/// its time at the extremes and never in between.
const RELEASE: f32 = 0.12;

/// Reads the tap and turns it into bar heights.
///
/// Owned by the engine and used from the control side only: it allocates on the
/// way in and holds a plan, neither of which the callback may go near.
pub(crate) struct Visualiser {
    fft: Arc<dyn Fft<f32>>,
    rate: u32,
    channels: usize,
    /// Everything the tap had, newest last. Reused so a reading allocates
    /// nothing.
    drained: Vec<f32>,
    /// One window of it, summed to mono and shaped.
    window: Vec<Complex<f32>>,
    /// The shape, worked out once.
    taper: Vec<f32>,
    /// Where each bar stands, so it can fall rather than jump.
    levels: Vec<f32>,
}

impl Visualiser {
    /// Builds a reader for one output format and a fixed number of bars.
    pub(crate) fn new(rate: u32, channels: u16, bars: usize) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(WINDOW);

        Self {
            fft,
            rate,
            channels: usize::from(channels.max(1)),
            drained: Vec::new(),
            window: vec![Complex { re: 0.0, im: 0.0 }; WINDOW],
            // A Hann window. Without it the ends of the block are a step, and a
            // step is broadband: every bar would carry a little of every other.
            taper: (0..WINDOW)
                .map(|index| {
                    let phase = std::f32::consts::TAU * index as f32 / (WINDOW as f32 - 1.0);
                    0.5 - 0.5 * phase.cos()
                })
                .collect(),
            levels: vec![0.0; bars],
        }
    }

    /// Fills `bars` with heights from zero to one.
    ///
    /// Returns false when there was nothing new to look at, so a caller can
    /// leave the last picture up rather than blink it away.
    pub(crate) fn read(&mut self, shared: &Shared, bars: &mut [f32]) -> bool {
        if bars.len() != self.levels.len() {
            self.levels = vec![0.0; bars.len()];
        }

        let taken = self.drain(shared);
        if taken < WINDOW * self.channels {
            // Not enough yet, or nothing playing. Let the bars fall to rest
            // rather than freeze mid-air.
            return self.settle(bars);
        }

        self.fill_window(taken);
        self.fft.process(&mut self.window);
        self.bucket(bars);
        true
    }

    /// Empties the tap into `drained` and returns how much is there.
    ///
    /// Everything, not one window: the callback runs far ahead of thirty
    /// readings a second, and what matters is the newest sound rather than the
    /// oldest. Draining it also leaves the ring room for what comes next.
    fn drain(&mut self, shared: &Shared) -> usize {
        let capacity = shared.tap.capacity();
        if self.drained.len() < capacity {
            self.drained.resize(capacity, 0.0);
        }

        let mut total = 0;
        loop {
            let taken = shared.tap.pop(&mut self.drained[total..]);
            if taken == 0 {
                break;
            }
            total += taken;
            if total >= self.drained.len() {
                break;
            }
        }
        total
    }

    /// Sums the newest window to mono and shapes it.
    fn fill_window(&mut self, taken: usize) {
        let newest = taken - WINDOW * self.channels;

        for index in 0..WINDOW {
            let frame = newest + index * self.channels;
            let mut sum = 0.0;
            for channel in 0..self.channels {
                sum += self.drained[frame + channel];
            }

            self.window[index] = Complex {
                re: sum / self.channels as f32 * self.taper[index],
                im: 0.0,
            };
        }
    }

    /// Folds the transform into bars, spaced by octave rather than by hertz.
    ///
    /// A linear spacing gives the top octave half the bars and the bottom two
    /// none: the ear hears ratios, and so does this.
    fn bucket(&mut self, bars: &mut [f32]) {
        let bin_hz = self.rate as f32 / WINDOW as f32;
        let span = (HIGHEST_HZ / LOWEST_HZ).log2();

        for (index, level) in self.levels.iter_mut().enumerate() {
            let from = LOWEST_HZ * 2.0_f32.powf(span * index as f32 / bars.len() as f32);
            let to = LOWEST_HZ * 2.0_f32.powf(span * (index + 1) as f32 / bars.len() as f32);

            let first = ((from / bin_hz).floor() as usize).max(1);
            let last = ((to / bin_hz).ceil() as usize).min(WINDOW / 2 - 1);

            let mut loudest = 0.0_f32;
            for bin in first..=last.max(first) {
                loudest = loudest.max(self.window[bin].norm());
            }

            // Decibels, because that is how loudness is heard and how a bar
            // that means anything has to be scaled.
            let db = 20.0 * (loudest / (WINDOW as f32 / 4.0)).max(1e-6).log10();
            let height = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);

            *level += (height - *level) * if height > *level { ATTACK } else { RELEASE };
        }

        bars.copy_from_slice(&self.levels);
    }

    /// Lets every bar fall towards rest, and says whether any of them moved.
    fn settle(&mut self, bars: &mut [f32]) -> bool {
        let mut moving = false;
        for level in &mut self.levels {
            if *level > 0.001 {
                *level *= 1.0 - RELEASE;
                moving = true;
            } else {
                *level = 0.0;
            }
        }

        bars.copy_from_slice(&self.levels);
        moving
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use super::{Visualiser, WINDOW};
    use crate::audio::stream::Shared;

    const RATE: u32 = 44_100;

    /// A tap filled with a steady tone, as the callback would have left it.
    fn playing(frequency_hz: f32) -> Arc<Shared> {
        let shared = Arc::new(Shared::new(RATE, 1));
        shared.tapping.store(true, Ordering::Relaxed);

        let step = std::f32::consts::TAU * frequency_hz / RATE as f32;
        let tone: Vec<f32> = (0..WINDOW * 2)
            .map(|sample| (step * sample as f32).sin())
            .collect();
        shared.tap.push(&tone);
        shared
    }

    /// Reads until the bars have caught up with what is playing.
    fn settled(visualiser: &mut Visualiser, shared: &Arc<Shared>, bars: &mut [f32]) {
        for _ in 0..30 {
            visualiser.read(shared, bars);
            let step = std::f32::consts::TAU * 1_000.0 / RATE as f32;
            let more: Vec<f32> = (0..WINDOW).map(|s| (step * s as f32).sin()).collect();
            let _ = shared.tap.push(&more);
        }
    }

    #[test]
    fn a_tone_lights_the_bar_it_belongs_to_and_leaves_the_others_alone() {
        let shared = playing(1_000.0);
        let mut visualiser = Visualiser::new(RATE, 1, 12);
        let mut bars = [0.0; 12];

        settled(&mut visualiser, &shared, &mut bars);

        // Twelve bars from 40 Hz to 16 kHz is half an octave each; 1 kHz lands
        // in the eighth.
        let loudest = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).expect("no NaN"))
            .expect("twelve bars")
            .0;
        assert!(
            (6..=8).contains(&loudest),
            "a kilohertz tone lit bar {loudest}: {bars:?}"
        );
        assert!(
            bars[0] < bars[loudest] / 2.0,
            "and the bottom of the spectrum stayed down: {bars:?}"
        );
    }

    #[test]
    fn silence_lets_every_bar_fall_to_rest() {
        let shared = playing(1_000.0);
        let mut visualiser = Visualiser::new(RATE, 1, 12);
        let mut bars = [0.0; 12];
        settled(&mut visualiser, &shared, &mut bars);

        // Nothing more arrives: the tap runs dry and the bars come down.
        for _ in 0..200 {
            visualiser.read(&shared, &mut bars);
        }

        assert!(
            bars.iter().all(|height| *height == 0.0),
            "something is still up: {bars:?}"
        );
        assert!(
            !visualiser.read(&shared, &mut bars),
            "and a reading with nothing to say says so"
        );
    }

    /// What one reading costs, printed rather than asserted.
    ///
    /// PROJECT_MASTER 2.9 asks that the visualiser not grow the processor's
    /// load noticeably, and that is a measurement rather than a claim. Run it:
    ///
    /// ```text
    /// cargo test -p cadenza-infra --lib what_a_reading_costs -- --ignored --nocapture
    /// ```
    ///
    /// Ignored because a timing on a busy machine is a number about the
    /// machine, and a test that fails when a build runs beside it is a test
    /// nobody trusts.
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn what_a_reading_costs() {
        let shared = playing(1_000.0);
        let mut visualiser = Visualiser::new(RATE, 1, 12);
        let mut bars = [0.0; 12];

        let step = std::f32::consts::TAU * 1_000.0 / RATE as f32;
        let block: Vec<f32> = (0..WINDOW).map(|s| (step * s as f32).sin()).collect();

        let rounds = 1_000;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            shared.tap.push(&block);
            visualiser.read(&shared, &mut bars);
        }
        let each = started.elapsed() / rounds;

        println!("one reading: {each:?}");
        println!(
            "thirty a second: {:.4}% of one core",
            each.as_secs_f64() * 30.0 * 100.0
        );
    }

    #[test]
    fn a_bar_rises_faster_than_it_falls() {
        let shared = playing(1_000.0);
        let mut visualiser = Visualiser::new(RATE, 1, 12);
        let mut bars = [0.0; 12];

        // One reading, on a tap that already holds the tone.
        assert!(visualiser.read(&shared, &mut bars));
        let lit = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).expect("no NaN"))
            .expect("twelve bars")
            .0;
        let after_one = bars[lit];
        assert!(after_one > 0.0, "the first reading already shows something");

        // Nothing more arrives. One step down has to be smaller than the step
        // up was, which is what stops a row of bars flickering.
        visualiser.read(&shared, &mut bars);
        let fell = after_one - bars[lit];

        assert!(fell > 0.0, "it came down");
        assert!(
            fell < after_one,
            "and by less than it went up: rose {after_one}, fell {fell}"
        );
    }
}
