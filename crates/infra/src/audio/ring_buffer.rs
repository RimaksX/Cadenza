//! The lock-free ring between the decoder and the audio callback.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// A single-producer, single-consumer ring of `f32` samples.
///
/// The audio callback may not allocate, block on a mutex or perform IO , which
/// rules out a `Mutex<VecDeque<f32>>` and leaves a ring whose two ends never
/// write the same index: the producer owns `write` and the consumer owns
/// `read`.
///
/// Samples are held as their bit patterns in relaxed atomics rather than behind
/// an `UnsafeCell`. On x86 a relaxed load is a plain move, so the cost is a
/// missed vectorisation in the copy — and in exchange the whole audio path stays
/// inside safe Rust, which is the rest of this crate's default and the only way
/// the callback's contract can be checked by a reader rather than trusted.
///
/// ponytail: per-sample atomics. If a profile ever shows the copy mattering, the
/// upgrade is an `UnsafeCell<[f32]>` behind exactly this API.
#[derive(Debug)]
pub struct SampleRing {
    slots: Box<[AtomicU32]>,
    /// `slots.len() - 1`. The capacity is a power of two so that wrapping an
    /// index is a mask rather than a division, once per sample.
    mask: usize,
    /// Samples per frame. Every transfer is truncated to a whole number of
    /// frames, so a partial write can never leave the two ends disagreeing about
    /// which channel a sample belongs to.
    frame_size: usize,
    /// Written by the producer only.
    write: AtomicUsize,
    /// Written by the consumer only.
    read: AtomicUsize,
}

impl SampleRing {
    /// Builds a ring holding at least `frames` frames of `channels` channels.
    ///
    /// The capacity is rounded up to a power of two, so the real capacity is
    /// usually larger than asked for. It is a pre-buffer, not a latency budget:
    /// output latency is cpal's buffer, and this only decides how far ahead the
    /// decoder is allowed to run.
    pub fn new(frames: usize, channels: u16) -> Self {
        let frame_size = usize::from(channels).max(1);
        let capacity = (frames * frame_size)
            .max(frame_size * 2)
            .next_power_of_two();

        Self {
            slots: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            mask: capacity - 1,
            frame_size,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    /// Total number of samples the ring can hold.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Samples currently waiting to be played.
    ///
    /// Safe to call from either end; the answer is a moment old by the time it
    /// is returned, and is only ever used to decide whether more work is worth
    /// starting.
    pub fn available(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    /// True when nothing is left to play.
    pub fn is_empty(&self) -> bool {
        self.available() == 0
    }

    /// Copies as much of `samples` into the ring as fits, and reports how much.
    ///
    /// Producer side only. A short result means the ring is full and the caller
    /// must keep the rest.
    pub fn push(&self, samples: &[f32]) -> usize {
        let write = self.write.load(Ordering::Relaxed);
        let used = write.wrapping_sub(self.read.load(Ordering::Acquire));
        let free = self.capacity() - used;
        let count = self.whole_frames(free.min(samples.len()));

        for (offset, sample) in samples[..count].iter().enumerate() {
            self.slots[write.wrapping_add(offset) & self.mask]
                .store(sample.to_bits(), Ordering::Relaxed);
        }

        // Release: the samples above must be visible before the consumer is told
        // that they exist.
        self.write
            .store(write.wrapping_add(count), Ordering::Release);
        count
    }

    /// Fills the front of `out` from the ring, and reports how much was written.
    ///
    /// Consumer side only, and called from the audio callback: no allocation, no
    /// locking, nothing that can block. A short result is an underrun, and the
    /// caller is responsible for the silence that has to follow it.
    pub fn pop(&self, out: &mut [f32]) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let used = self.write.load(Ordering::Acquire).wrapping_sub(read);
        let count = self.whole_frames(used.min(out.len()));

        for (offset, slot) in out[..count].iter_mut().enumerate() {
            *slot = f32::from_bits(
                self.slots[read.wrapping_add(offset) & self.mask].load(Ordering::Relaxed),
            );
        }

        self.read.store(read.wrapping_add(count), Ordering::Release);
        count
    }

    /// Discards everything waiting.
    ///
    /// Consumer side only — it moves the read index, which is why a seek is
    /// carried out by the callback rather than by the decoder that asked for it.
    pub fn drain(&self) {
        self.read
            .store(self.write.load(Ordering::Acquire), Ordering::Release);
    }

    /// Rounds a sample count down to a whole number of frames.
    fn whole_frames(&self, samples: usize) -> usize {
        samples - samples % self.frame_size
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use super::SampleRing;

    #[test]
    fn what_goes_in_comes_out_in_order() {
        let ring = SampleRing::new(8, 2);
        assert_eq!(ring.push(&[1.0, 2.0, 3.0, 4.0]), 4);

        let mut out = [0.0; 4];
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert!(ring.is_empty());
    }

    #[test]
    fn a_full_ring_takes_what_fits_and_no_more() {
        let ring = SampleRing::new(4, 2); // eight samples
        let samples = [0.5; 12];

        assert_eq!(ring.push(&samples), ring.capacity(), "fills to capacity");
        assert_eq!(ring.push(&samples), 0, "and then refuses everything");
    }

    #[test]
    fn transfers_never_split_a_frame() {
        let ring = SampleRing::new(4, 2); // four stereo frames
        assert_eq!(ring.push(&[0.0; 7]), 6, "an odd tail is held back");

        let mut out = [0.0; 5];
        assert_eq!(ring.pop(&mut out), 4, "and an odd read is trimmed");
    }

    #[test]
    fn indices_wrap_without_losing_a_sample() {
        let ring = SampleRing::new(4, 2);
        let mut out = [0.0; 2];

        // Ten laps of a ring that holds four frames: every index is reused.
        for lap in 0..10_u8 {
            let value = f32::from(lap);
            assert_eq!(ring.push(&[value, value]), 2);
            assert_eq!(ring.pop(&mut out), 2);
            assert_eq!(out, [value, value]);
        }
    }

    #[test]
    fn draining_throws_away_what_was_queued() {
        let ring = SampleRing::new(8, 2);
        ring.push(&[1.0; 8]);
        ring.drain();

        assert!(ring.is_empty());
        assert_eq!(ring.pop(&mut [0.0; 8]), 0);
    }

    #[test]
    fn a_producer_and_a_consumer_on_two_threads_agree_on_every_sample() {
        const FRAMES: usize = 50_000;

        let ring = Arc::new(SampleRing::new(64, 2));
        let finished = Arc::new(AtomicBool::new(false));

        let producer = {
            let ring = Arc::clone(&ring);
            let finished = Arc::clone(&finished);
            thread::spawn(move || {
                let mut frame = 0u32;
                while (frame as usize) < FRAMES {
                    let value = frame as f32;
                    if ring.push(&[value, value]) == 2 {
                        frame += 1;
                    } else {
                        thread::yield_now();
                    }
                }
                finished.store(true, Ordering::Release);
            })
        };

        let mut expected = 0u32;
        let mut out = [0.0; 2];
        while (expected as usize) < FRAMES {
            if ring.pop(&mut out) == 2 {
                assert_eq!(out, [expected as f32; 2], "frame {expected} arrived intact");
                expected += 1;
            } else {
                assert!(
                    !finished.load(Ordering::Acquire) || !ring.is_empty(),
                    "the producer finished but frames went missing"
                );
                thread::yield_now();
            }
        }

        producer.join().expect("producer thread");
    }
}
