//! Runtime observers: something outside the run that sees every runtime event and changes
//! nothing.
//!
//! An observer gets each [`RuntimeEvent`] by shared reference, after the runtime decided and did
//! what the event records, and returns nothing. It cannot reach a program, a mailbox, a timer or
//! the trace, so a run with an observer is the same run as without one
//! ([`crate::RunOutcome::same_run`]). Program step events (the stepper's own observer) are a
//! separate hook: [`crate::ProgramInstance::observer`].

use crate::trace::RuntimeEvent;

/// Sees every runtime event of a run, in sequence order.
pub trait RuntimeObserver {
    /// Whether the runtime should build events for this observer. With `false` and the trace off,
    /// the runtime builds no event at all.
    fn observes(&self) -> bool {
        true
    }

    /// One event, after it happened.
    fn on_event(&mut self, event: &RuntimeEvent);
}

/// No runtime observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoRuntimeObserver;

impl RuntimeObserver for NoRuntimeObserver {
    fn observes(&self) -> bool {
        false
    }

    fn on_event(&mut self, _event: &RuntimeEvent) {}
}

/// Keeps every event it sees: for tests and tools.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventCollector {
    /// Every event seen, in order.
    pub events: Vec<RuntimeEvent>,
}

impl RuntimeObserver for EventCollector {
    fn on_event(&mut self, event: &RuntimeEvent) {
        self.events.push(event.clone());
    }
}
