//! Asking Windows to schedule a thread out of the way.

use cadenza_core::Result;
use cadenza_core::domain::ports::system_priority::{PriorityClass, SystemPriorityPort};

/// Thread scheduling, as far as safe Rust can reach it.
///
/// Which, today, is not at all: `SetThreadPriority` needs either `unsafe` or a
/// dependency taken for one function. Neither is worth it, because the promise
/// — background work under about a fifth of the machine — is kept by
/// `analysis_policy`'s duty cycle instead, and a share of the clock does not
/// depend on a scheduler agreeing with it.
///
/// **So this reports success and changes nothing**, which is what
/// [`SystemPriorityPort`] asks of an implementation that cannot lower priority.
/// It exists as the seam.
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
