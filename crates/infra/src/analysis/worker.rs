//! The thread that does the analysing, and the leash it is kept on.
//!
//! One file at a time, a rest after each, and a check between every step for
//! whether it has been told to stop. Nothing here decides *what* to analyse —
//! that is [`AnalysisService`]'s business — only how much of the machine the
//! work is allowed to take (PROJECT_MASTER 2.11, 12.1).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use cadenza_core::application::services::AnalysisService;
use cadenza_core::domain::policies::analysis_policy::{
    MAX_NAP, SHARE_WHEN_IDLE, SHARE_WHILE_PLAYING, nap_after,
};
use cadenza_core::domain::ports::system_priority::{PriorityClass, SystemPriorityPort};

/// Whether the machine has something better to do.
///
/// A closure rather than a handle on playback, because "busy" is the
/// application's judgement and this thread only needs the answer. Called
/// between files, never during one.
pub type Busy = Arc<dyn Fn() -> bool + Send + Sync>;

/// A background analyser, running until it is dropped.
pub struct AnalysisWorker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl AnalysisWorker {
    /// Starts analysing in the background.
    ///
    /// Returns immediately. The thread it starts is stopped and joined when the
    /// returned worker is dropped, which is what keeps a closing window from
    /// leaving a thread holding a database connection behind it.
    pub fn start(
        service: Arc<AnalysisService>,
        priority: Arc<dyn SystemPriorityPort>,
        busy: Busy,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));

        let handle = thread::Builder::new()
            .name("cadenza-analysis".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || run(&service, priority.as_ref(), &busy, &stop)
            })
            .ok();

        Self { stop, handle }
    }

    /// Asks the thread to finish the file it is on and stop.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AnalysisWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The loop itself.
fn run(
    service: &AnalysisService,
    priority: &dyn SystemPriorityPort,
    busy: &Busy,
    stop: &AtomicBool,
) {
    // Best effort, and the port says so: a platform that will not lower a
    // thread's priority is a performance problem, not a correctness one, and
    // the share below is what actually keeps the promise.
    let _ = priority.set_current_thread(PriorityClass::Background);

    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();

        let worked = match service.run_next() {
            Ok(Some(_)) => true,
            Ok(None) => {
                // Nothing waiting. Either there is more to queue, or the
                // library is done and the thread should stop asking.
                match service.top_up() {
                    Ok(0) | Err(_) => false,
                    Ok(_) => continue,
                }
            }
            // A failure that reaches here is the database, not a file: a file
            // that will not analyse is recorded against its job and reported as
            // success. Resting and trying again is the only useful answer.
            Err(_) => false,
        };

        let share = if (busy)() {
            SHARE_WHILE_PLAYING
        } else {
            SHARE_WHEN_IDLE
        };

        // Nothing to do means nothing to pay for either — but also no reason to
        // spin: the longest nap is what an idle library costs.
        let nap = if worked {
            nap_after(started.elapsed(), share)
        } else {
            MAX_NAP
        };

        // Slept in slices so that closing the window does not wait out the rest.
        let woken = Instant::now();
        while woken.elapsed() < nap && !stop.load(Ordering::Relaxed) {
            thread::sleep(core::time::Duration::from_millis(50).min(nap));
        }
    }
}
