//! The ob-serve-a-bowl runtime observer (feature `bowl`).
//!
//! Each runtime event becomes one or more `WakeKey64` packets published to an
//! `ObservableWakeRuntime`; the `WakeReport` each publish returns is dropped, so nothing flows
//! back into the run. A step attempt publishes [`BowlPhase::Prepared`], then either
//! [`BowlPhase::Refused`] (and [`BowlPhase::Held`] or [`BowlPhase::Dropped`] by policy) or
//! [`BowlPhase::Admitted`], [`BowlPhase::Committed`] and one packet per action. A refused step
//! never publishes `Admitted` or `Committed`, and carries [`REFUSED_BIT`] in its field mask, never
//! [`COMMITTED_BIT`].
//!
//! Packet layout: `phase` is the [`BowlPhase`]; `reason` is Scope's matching wake reason (`manual`
//! for steps, `inbox` for deliveries, `timer`, `cancel`, `resume` for a released place, `shutdown`
//! for a finished program); `field_mask` holds [`COMMITTED_BIT`], [`REFUSED_BIT`], [`HELD_BIT`];
//! the payload holds the program slot (`slot`, `u32::MAX` when none), the event's sequence number
//! (`generation`, low 32 bits), the message id (`offset`, low 32 bits, `u32::MAX` when none) and a
//! count (`len`: refusal reasons, deliveries, woken tasks or dropped messages).
//!
//! Program step events go to big-bad-bool-program's own `BowlObserver`
//! (`big_bad_bool::program::BowlObserver`), set per program with
//! [`crate::ProgramInstance::observer`]; it sees committed steps only, because `prepare` does not
//! call the program observer.

use ob_serve_a_bowl::{
    ObservablePacket, ObservableWakeRuntime, PayloadRef, WakeKey64, WakeProfileCode, WakeReasonCode,
};

use crate::observer::RuntimeObserver;
use crate::trace::{
    ActionRecord, Admission, Delivery, Disposition, RuntimeEvent, RuntimeEventKind,
};

/// Field-mask bit of a committed step's packets.
pub const COMMITTED_BIT: u16 = 1 << 0;
/// Field-mask bit of a refused step's packets.
pub const REFUSED_BIT: u16 = 1 << 1;
/// Field-mask bit of a packet about an input a policy holds.
pub const HELD_BIT: u16 = 1 << 2;

/// The `phase` of a published packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BowlPhase {
    /// `RuntimeEventKind::Epoch`.
    Epoch = 1,
    /// `RuntimeEventKind::Injected`.
    Injected = 2,
    /// A step attempt was prepared.
    Prepared = 3,
    /// Admission refused it.
    Refused = 4,
    /// Admission admitted it.
    Admitted = 5,
    /// It committed.
    Committed = 6,
    /// A committed `Send` routed its deliveries.
    Routed = 7,
    /// A committed `Timer` armed a logical timer.
    TimerArmed = 8,
    /// A committed `StartChild` started a child.
    ChildStarted = 9,
    /// A committed `CancelChild` asked Scope to cancel a child.
    CancelRequested = 10,
    /// A committed `External` or `Trace`.
    Handed = 11,
    /// A refused input is held by its policy.
    Held = 12,
    /// A refused input was dropped by its policy (backlog full, attempts exhausted).
    Dropped = 13,
    /// `RuntimeEventKind::TimerFired`.
    TimerFired = 14,
    /// `RuntimeEventKind::TimerWaiting`.
    TimerWaiting = 15,
    /// `RuntimeEventKind::ChildTick`.
    ChildTick = 16,
    /// `RuntimeEventKind::ChildEnded`.
    ChildEnded = 17,
    /// `RuntimeEventKind::Released`.
    Released = 18,
    /// `RuntimeEventKind::Finished`.
    Finished = 19,
}

/// Publishes every runtime event to an ob-serve-a-bowl runtime.
#[derive(Default)]
pub struct BowlRuntimeObserver {
    runtime: ObservableWakeRuntime,
    published: usize,
}

impl std::fmt::Debug for BowlRuntimeObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BowlRuntimeObserver")
            .field("published", &self.published)
            .finish_non_exhaustive()
    }
}

impl BowlRuntimeObserver {
    /// An adapter over `runtime`. Register ob-serve-a-bowl observers on it first.
    #[must_use]
    pub fn new(runtime: ObservableWakeRuntime) -> Self {
        Self {
            runtime,
            published: 0,
        }
    }

    /// The runtime, to read what was delivered.
    #[must_use]
    pub const fn runtime(&self) -> &ObservableWakeRuntime {
        &self.runtime
    }

    /// The runtime, to drain delivered events.
    pub fn runtime_mut(&mut self) -> &mut ObservableWakeRuntime {
        &mut self.runtime
    }

    /// Packets published so far.
    #[must_use]
    pub const fn published(&self) -> usize {
        self.published
    }

    // One packet: its key fields, then its payload fields, as the layout above lists them.
    #[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
    fn publish(
        &mut self,
        event: &RuntimeEvent,
        phase: BowlPhase,
        reason: WakeReasonCode,
        mask: u16,
        slot: Option<u32>,
        message: Option<u64>,
        len: usize,
    ) {
        let key = WakeKey64::new(
            reason.as_u64(),
            WakeProfileCode::InlineQueue.as_u64(),
            3,
            u64::from(mask),
            phase as u64,
            0,
        );
        let payload = PayloadRef::new(
            slot.unwrap_or(u32::MAX),
            event.seq as u32,
            message.map_or(u32::MAX, |m| m as u32),
            u32::try_from(len).unwrap_or(u32::MAX),
        );
        let _report = self.runtime.publish(ObservablePacket::new(key, payload));
        self.published += 1;
    }
}

impl RuntimeObserver for BowlRuntimeObserver {
    #[allow(clippy::too_many_lines)]
    fn on_event(&mut self, event: &RuntimeEvent) {
        use WakeReasonCode as R;
        let delivered = |d: &Delivery| match d {
            Delivery::Delivered { to, message } => (Some(to.0), Some(message.0)),
            Delivery::Refused { message, .. } => (None, Some(message.0)),
        };
        match &event.kind {
            RuntimeEventKind::Epoch { .. } => {
                self.publish(event, BowlPhase::Epoch, R::Manual, 0, None, None, 0);
            }
            RuntimeEventKind::Injected { delivery } => {
                let (to, m) = delivered(delivery);
                self.publish(event, BowlPhase::Injected, R::Inbox, 0, to, m, 1);
            }
            RuntimeEventKind::Step(s) => {
                let (slot, m) = (Some(s.program.0), Some(s.message.0));
                self.publish(
                    event,
                    BowlPhase::Prepared,
                    R::Manual,
                    0,
                    slot,
                    m,
                    s.results.len(),
                );
                match &s.admission {
                    Admission::Refused(reasons) => {
                        let held = if s.disposition.is_held() { HELD_BIT } else { 0 };
                        let mask = REFUSED_BIT | held;
                        self.publish(
                            event,
                            BowlPhase::Refused,
                            R::Manual,
                            mask,
                            slot,
                            m,
                            reasons.len(),
                        );
                        match s.disposition {
                            d if d.is_held() => {
                                self.publish(event, BowlPhase::Held, R::Manual, mask, slot, m, 1);
                            }
                            Disposition::BacklogFull { .. } | Disposition::Exhausted { .. } => {
                                self.publish(
                                    event,
                                    BowlPhase::Dropped,
                                    R::Manual,
                                    REFUSED_BIT,
                                    slot,
                                    m,
                                    1,
                                );
                            }
                            _ => {}
                        }
                    }
                    Admission::Admitted => {
                        let c = COMMITTED_BIT;
                        self.publish(
                            event,
                            BowlPhase::Admitted,
                            R::Manual,
                            c,
                            slot,
                            m,
                            s.effects.len(),
                        );
                        self.publish(
                            event,
                            BowlPhase::Committed,
                            R::Manual,
                            c,
                            slot,
                            m,
                            s.results.len(),
                        );
                        for a in &s.actions {
                            let (phase, reason, len) = match a {
                                ActionRecord::Sent { deliveries, .. } => {
                                    (BowlPhase::Routed, R::Inbox, deliveries.len())
                                }
                                ActionRecord::TimerArmed { .. } => {
                                    (BowlPhase::TimerArmed, R::Timer, 1)
                                }
                                ActionRecord::ChildStarted { .. } => {
                                    (BowlPhase::ChildStarted, R::Spawn, 1)
                                }
                                ActionRecord::ChildCancelRequested { .. } => {
                                    (BowlPhase::CancelRequested, R::Cancel, 1)
                                }
                                ActionRecord::External { .. } | ActionRecord::Trace { .. } => {
                                    (BowlPhase::Handed, R::Manual, 1)
                                }
                            };
                            self.publish(event, phase, reason, c, slot, m, len);
                        }
                    }
                }
            }
            RuntimeEventKind::TimerFired {
                program, delivery, ..
            } => {
                let (_, m) = delivered(delivery);
                self.publish(
                    event,
                    BowlPhase::TimerFired,
                    R::Timer,
                    0,
                    Some(program.0),
                    m,
                    1,
                );
            }
            RuntimeEventKind::TimerWaiting {
                program, message, ..
            } => {
                let (slot, m) = (Some(program.0), Some(message.0));
                self.publish(
                    event,
                    BowlPhase::TimerWaiting,
                    R::Timer,
                    HELD_BIT,
                    slot,
                    m,
                    1,
                );
            }
            RuntimeEventKind::ChildTick {
                program, delivery, ..
            } => {
                let (_, m) = delivered(delivery);
                self.publish(
                    event,
                    BowlPhase::ChildTick,
                    R::Inbox,
                    0,
                    Some(program.0),
                    m,
                    1,
                );
            }
            RuntimeEventKind::ChildEnded {
                program, cancelled, ..
            } => {
                let reason = if *cancelled { R::Cancel } else { R::Manual };
                self.publish(
                    event,
                    BowlPhase::ChildEnded,
                    reason,
                    0,
                    Some(program.0),
                    None,
                    1,
                );
            }
            RuntimeEventKind::Released { woken, .. } => {
                self.publish(
                    event,
                    BowlPhase::Released,
                    R::Resume,
                    0,
                    None,
                    None,
                    woken.len(),
                );
            }
            RuntimeEventKind::Finished {
                program,
                dropped,
                held,
                ..
            } => {
                let len = dropped.len() + held.len();
                self.publish(
                    event,
                    BowlPhase::Finished,
                    R::Shutdown,
                    0,
                    Some(program.0),
                    None,
                    len,
                );
            }
        }
    }
}
