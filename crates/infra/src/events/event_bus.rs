//! A synchronous, in-process event bus.

use std::sync::{Arc, PoisonError, RwLock};

use cadenza_core::domain::ports::event_bus::{DomainEvent, EventBusPort, EventHandler};

/// Fans events out to every subscriber on the publishing thread.
///
/// Synchronous and in-process because that is all Cadenza needs: there is one
/// application, no network, and the subscribers are the UI and a handful of
/// background workers. A queue and a delivery thread would add ordering
/// questions and a shutdown problem in exchange for nothing.
/// A registered subscriber.
///
/// `Arc` rather than the port's `Box` so that [`InProcessEventBus::publish`] can
/// take a cheap snapshot and drop the lock before calling anything.
type Subscriber = Arc<dyn Fn(&DomainEvent) + Send + Sync>;

#[derive(Default)]
pub struct InProcessEventBus {
    handlers: RwLock<Vec<Subscriber>>,
}

impl InProcessEventBus {
    /// A bus with no subscribers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many handlers are registered.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.handlers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl EventBusPort for InProcessEventBus {
    /// Delivers to every subscriber registered at the moment of the call.
    ///
    /// The lock is released before any handler runs. Holding it would deadlock
    /// the moment a handler subscribed, published, or did anything else that
    /// reached back into the bus — and a handler doing that is a reasonable
    /// thing to write, not a bug to be documented around.
    fn publish(&self, event: DomainEvent) {
        let subscribers = {
            let handlers = self.handlers.read().unwrap_or_else(PoisonError::into_inner);
            handlers.clone()
        };

        for handler in subscribers {
            handler(&event);
        }
    }

    fn subscribe(&self, handler: EventHandler) {
        self.handlers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::from(handler));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::InProcessEventBus;
    use cadenza_core::domain::ids::ProfileId;
    use cadenza_core::domain::ports::event_bus::{DomainEvent, EventBusPort};

    #[test]
    fn every_subscriber_sees_every_event() {
        let bus = InProcessEventBus::new();
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));

        for counter in [Arc::clone(&first), Arc::clone(&second)] {
            bus.subscribe(Box::new(move |_event| {
                counter.fetch_add(1, Ordering::Relaxed);
            }));
        }

        bus.publish(DomainEvent::LibraryChanged);
        bus.publish(DomainEvent::QueueChanged);

        assert_eq!(first.load(Ordering::Relaxed), 2);
        assert_eq!(second.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_event_arrives_intact() {
        let bus = InProcessEventBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        bus.subscribe(Box::new(move |event| {
            recorder.lock().expect("not poisoned").push(event.clone());
        }));

        let profile = ProfileId::new();
        bus.publish(DomainEvent::ProfileSwitched(profile));

        assert_eq!(
            *seen.lock().expect("not poisoned"),
            vec![DomainEvent::ProfileSwitched(profile)]
        );
    }

    #[test]
    fn a_handler_may_subscribe_from_inside_a_handler() {
        // Holding the lock across delivery would deadlock here rather than fail
        // a test, so this is the case worth pinning down.
        let bus = Arc::new(InProcessEventBus::new());
        let inner = Arc::clone(&bus);

        bus.subscribe(Box::new(move |_event| {
            inner.subscribe(Box::new(|_| {}));
        }));

        bus.publish(DomainEvent::LibraryChanged);
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn publishing_with_no_subscribers_is_harmless() {
        InProcessEventBus::new().publish(DomainEvent::EqChanged);
    }
}
