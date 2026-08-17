//! Asking Windows to schedule a thread out of the way.

use cadenza_core::Result;
use cadenza_core::domain::ports::system_priority::{PriorityClass, SystemPriorityPort};

/// Thread scheduling, as far as safe Rust can reach it.
///
/// Which, today, is not at all: `SetThreadPriority` is a Win32 call, and
/// reaching it means either `unsafe` or a dependency taken for one function.
/// Neither is worth it, because the guarantee PROJECT_MASTER 2.11 actually asks
/// for — background work under about a fifth of the machine — is kept by
/// `analysis_policy`'s duty cycle instead: the worker rests four times as long
/// as it works, and a share of the clock is a promise that does not depend on a
/// scheduler agreeing with it.
///
/// So this reports success and changes nothing, which is exactly what
/// [`SystemPriorityPort`] says an implementation that cannot lower priority
/// should do. It exists as the seam: the day the call is worth making, it is
/// made here and nothing else moves (MASTER_ISSUES 47).
pub struct WindowsPriority;

impl SystemPriorityPort for WindowsPriority {
    fn set_current_thread(&self, _class: PriorityClass) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{PriorityClass, SystemPriorityPort, WindowsPriority};

    #[test]
    fn asking_for_a_lower_priority_is_never_an_error() {
        assert!(
            WindowsPriority
                .set_current_thread(PriorityClass::Background)
                .is_ok()
        );
        assert!(
            WindowsPriority
                .set_current_thread(PriorityClass::Normal)
                .is_ok()
        );
    }
}
