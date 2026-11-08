//! The trace: what each run did, as plain comparable data.
//!
//! Every [`RuntimeEvent`] carries its sequence number in the run and the Scope tick it happened
//! at. Two runs are the same run when their events, their Scope scheduler events, their external
//! effects, final digests and counts are equal ([`crate::RunOutcome::same_run`]); the first
//! difference is reported by [`crate::RunOutcome::diff`]. Nothing here holds a wall-clock time.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use big_bad_bool::program::{
    ContractId, DimensionId, Liaise, ProjectionState, StableDigest, StepResult, VerifiedProgram,
};
use scope::{CancelReason, TaskId, TaskState, Tick, Tracer, WakeReason};

use crate::effect::RuntimeEffect;
use crate::ids::{ChildKey, MessageId, Origin, Port, ProgramId, ProgramSlot, TimerId};

/// SHA-256 (stable_digest) of a program's committed state, bound to its spec digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateDigest(pub StableDigest);

/// The digest of `state` under `program`: the spec digest, then for every dimension in order its
/// word and its stack, read through `ProjectionState`'s public accessors.
#[must_use]
pub fn state_digest(program: &VerifiedProgram, state: &ProjectionState) -> StateDigest {
    let mut material = Vec::with_capacity(96);
    material.extend_from_slice(program.digest().as_str().as_bytes());
    for i in 0..program.spec().dimensions().len() {
        #[allow(clippy::cast_possible_truncation)]
        let d = DimensionId(i as u16);
        material.extend_from_slice(&state.word(d).to_le_bytes());
        let stack = state.stack(d);
        #[allow(clippy::cast_possible_truncation)]
        material.extend_from_slice(&(stack.len() as u32).to_le_bytes());
        for w in stack {
            material.extend_from_slice(&w.to_le_bytes());
        }
    }
    StateDigest(StableDigest::sha256(material))
}

/// Why the runtime did not do something a step, a timer, a child or an injection asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuntimeRefusal {
    /// The receiver's bounded mailbox has fewer free places than the step would fill.
    MailboxFull {
        /// The receiver.
        to: ProgramSlot,
        /// Deliveries the step would make to it.
        needed: usize,
        /// Free places.
        free: usize,
    },
    /// The step's sends would make more deliveries than the bound.
    FanOut {
        /// Deliveries over all its sends.
        observed: usize,
        /// The bound.
        permitted: usize,
    },
    /// The receiver halted (a `Stop` refusal) or was shut down: its mailbox is closed.
    TargetFinished {
        /// The receiver.
        to: ProgramSlot,
    },
    /// No route leaves this port.
    NoRoute {
        /// The port.
        port: Port,
    },
    /// The step asked for more runtime effects than the bound.
    EffectsPerStep {
        /// How many it asked for.
        observed: usize,
        /// The bound.
        permitted: usize,
    },
    /// The step would arm more timers than are free.
    TooManyTimers {
        /// Timers the step would arm.
        needed: usize,
        /// Free timer places.
        free: usize,
    },
    /// The step would start more children than are free.
    TooManyChildren {
        /// Children the step would start.
        needed: usize,
        /// Free child places.
        free: usize,
    },
    /// The program already has a live child with this key.
    ChildExists {
        /// The key.
        key: ChildKey,
    },
    /// The program has no live child with this key.
    ChildUnknown {
        /// The key.
        key: ChildKey,
    },
    /// The run's step budget is spent; the input was not prepared.
    StepBudget {
        /// The bound.
        permitted: u64,
    },
}

impl RuntimeRefusal {
    /// Whether no later event of the run can make the same step fit: too many effects or
    /// deliveries, no route, a finished receiver, an unknown child, the step budget. A refusal
    /// that is not final waits for a place the run frees by itself: a mailbox place, a timer
    /// place, a child place or a child key.
    #[must_use]
    pub const fn is_final(&self) -> bool {
        !matches!(
            self,
            Self::MailboxFull { .. }
                | Self::TooManyTimers { .. }
                | Self::TooManyChildren { .. }
                | Self::ChildExists { .. }
        )
    }

    /// The place whose release can make this refusal go away; `None` when it is final.
    #[must_use]
    pub const fn resource(&self) -> Option<Resource> {
        match self {
            Self::MailboxFull { to, .. } => Some(Resource::Mailbox(*to)),
            Self::TooManyTimers { .. } => Some(Resource::Timers),
            Self::TooManyChildren { .. } | Self::ChildExists { .. } => Some(Resource::Children),
            _ => None,
        }
    }
}

/// A place a held input waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    /// A place in this program's mailbox: freed when it takes a message, commits a held input
    /// or finishes.
    Mailbox(ProgramSlot),
    /// A timer place: freed when a timer delivers its message (or its parent finished).
    Timers,
    /// A child place or key: freed when a child activity ends.
    Children,
}

/// One delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Delivery {
    /// Queued in the receiver's Scope mailbox, which woke it (`WakeReason::Inbox`).
    Delivered {
        /// The receiver.
        to: ProgramSlot,
        /// The message.
        message: MessageId,
    },
    /// Not queued.
    Refused {
        /// The message that was not delivered.
        message: MessageId,
        /// Why.
        reason: RuntimeRefusal,
    },
}

/// What the runtime did for one runtime effect of a committed step, in effect order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ActionRecord {
    /// `Send`: one delivery per route, in route order.
    Sent {
        /// The port.
        port: Port,
        /// The deliveries.
        deliveries: Vec<Delivery>,
    },
    /// `Timer`: a Scope logical timer task was spawned.
    TimerArmed {
        /// The timer.
        timer: TimerId,
        /// The message it will deliver.
        message: MessageId,
        /// Rounds after the timer task's first poll.
        after: u64,
    },
    /// `StartChild`: a Scope child task was spawned.
    ChildStarted {
        /// The child's key.
        key: ChildKey,
    },
    /// `CancelChild`: Scope's cancel was requested for the child's task (or, before the child's
    /// first poll, when its task id is not yet known, a flag it reads then).
    ChildCancelRequested {
        /// The child's key.
        key: ChildKey,
        /// The child's Scope task, when known.
        task: Option<TaskId>,
    },
    /// `External`: handed to the application.
    External {
        /// The tag.
        tag: u32,
        /// The word.
        word: u128,
    },
    /// `Trace`.
    Trace {
        /// The mark.
        tag: u32,
    },
}

/// One `StepResult` of one prepared step, as data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StepSummary {
    /// The input position.
    pub position: u32,
    /// The contracts that actualized together; empty when nothing committed.
    pub committed: Vec<ContractId>,
    /// The emitted effect words, `(dimension, word)` in dimension order.
    pub effects: Vec<(DimensionId, u128)>,
    /// The refusals' liaise codes (`BBBC0101` ..), in the stepper's order.
    pub refusals: Vec<String>,
}

impl StepSummary {
    pub(crate) fn of(result: &StepResult) -> Self {
        Self {
            position: result.position,
            committed: result.committed.clone(),
            effects: result
                .effects
                .iter()
                .map(|e| (e.dimension, e.word))
                .collect(),
            refusals: result.refusals.iter().map(Liaise::code).collect(),
        }
    }

    /// Whether this position committed a contract.
    #[must_use]
    pub fn is_committed(&self) -> bool {
        !self.committed.is_empty()
    }
}

/// Whether the runtime admitted a prepared step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Admission {
    /// Every runtime effect fit: the step committed, then the actions ran.
    Admitted,
    /// Something did not fit: the preparation was dropped, nothing changed, nothing ran. Every
    /// reason, in admission's order.
    Refused(Vec<RuntimeRefusal>),
}

impl Admission {
    /// Whether the step committed.
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

/// Where the input of a step attempt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSource {
    /// Taken from the program's mailbox in this poll.
    Mailbox,
    /// The head of the `Retry` backlog; `attempt` counts every attempt of this input, from 1.
    Backlog {
        /// This attempt's number (the mailbox attempt was 1).
        attempt: u32,
    },
    /// The input `Backpressure` held.
    Pending {
        /// This attempt's number (the mailbox attempt was 1).
        attempt: u32,
    },
}

/// What became of the input of one step attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Disposition {
    /// Admitted and committed.
    Committed,
    /// Refused and consumed: under `Consume`, or for a final refusal under any policy.
    Consumed {
        /// Whether a reason was final (`RuntimeRefusal::is_final`).
        final_refusal: bool,
    },
    /// `Retry`: refused and put at the back of the backlog, which now holds `len` inputs.
    Backlogged {
        /// Backlog length after it.
        len: usize,
    },
    /// `Retry`: a backlog entry refused again keeps its place; retrying stops for this poll.
    Kept {
        /// Its attempts so far.
        attempts: u32,
    },
    /// `Retry`: refused with the backlog full; the input is dropped.
    BacklogFull {
        /// The backlog's capacity.
        capacity: usize,
    },
    /// `Retry`: refused for the `max_attempts`-th time; the input is dropped.
    Exhausted {
        /// Its attempts.
        attempts: u32,
    },
    /// `Backpressure`: refused and held; the program takes no new input until it commits or is
    /// consumed.
    Backpressured {
        /// Its attempts so far.
        attempts: u32,
    },
}

impl Disposition {
    /// Whether the input is still held after this attempt.
    #[must_use]
    pub const fn is_held(self) -> bool {
        matches!(
            self,
            Self::Backlogged { .. } | Self::Kept { .. } | Self::Backpressured { .. }
        )
    }
}

/// One step attempt: `prepare`, admission, and `commit` when admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    /// The program.
    pub program: ProgramSlot,
    /// Its durable id.
    pub program_id: ProgramId,
    /// Its Scope task.
    pub task: TaskId,
    /// Scope's poll sequence number for this poll (from the `Tracer` hook; 0 with trace off).
    pub poll: u64,
    /// The last wake Scope dequeued for this task before this poll (`None` with trace off).
    pub wake: Option<WakeReason>,
    /// The input.
    pub message: MessageId,
    /// Who made it.
    pub origin: Origin,
    /// Where it came from in this poll.
    pub source: InputSource,
    /// The program-local position given to the observation.
    pub position: u32,
    /// State digest before the attempt.
    pub pre: StateDigest,
    /// State digest after it: equal to `pre` when admission refused.
    pub post: StateDigest,
    /// What the preparation gave, committed or not; empty when the window is not yet complete,
    /// or when the step budget refused the input before `prepare`.
    pub results: Vec<StepSummary>,
    /// The runtime effects the prepared results ask for, from the effect table, in order.
    pub effects: Vec<RuntimeEffect>,
    /// The runtime's decision.
    pub admission: Admission,
    /// What the runtime did, after the commit; empty when refused.
    pub actions: Vec<ActionRecord>,
    /// What became of the input.
    pub disposition: Disposition,
}

/// What one runtime event records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEventKind {
    /// The injections of epoch `epoch` are next, then Scope runs until idle.
    Epoch {
        /// From 0.
        epoch: u32,
    },
    /// A message injected through the builder, before its epoch's rounds.
    Injected {
        /// The delivery.
        delivery: Delivery,
    },
    /// A step attempt.
    Step(StepRecord),
    /// A logical timer's tick came and it tried its delivery.
    TimerFired {
        /// The program that armed it.
        program: ProgramSlot,
        /// The timer.
        timer: TimerId,
        /// The delivery.
        delivery: Delivery,
    },
    /// A timer's tick came while its program's mailbox had no free place: it waits for one
    /// (`Resource::Mailbox`) and keeps its timer place until it delivers.
    TimerWaiting {
        /// The program that armed it.
        program: ProgramSlot,
        /// The timer.
        timer: TimerId,
        /// The message it holds.
        message: MessageId,
    },
    /// A child activity tried a delivery to its parent.
    ChildTick {
        /// The parent.
        program: ProgramSlot,
        /// The child.
        key: ChildKey,
        /// The delivery.
        delivery: Delivery,
    },
    /// A child activity left the scope by itself or by cancellation.
    ChildEnded {
        /// The parent.
        program: ProgramSlot,
        /// The child.
        key: ChildKey,
        /// Whether Scope's cancel flag (or the runtime's flag before its first poll) was set.
        cancelled: bool,
    },
    /// A place was released and every task on the runtime's list for it was woken, in the order
    /// they began waiting: a timer or child place (Scope `WakeReason::Manual`), or a mailbox place
    /// Scope's own capacity wake cannot give (`WakeReason::Capacity`: the place a `Backpressure`
    /// input holds, or a step needing several places). A mailbox place a waiter registered for in
    /// Scope is held for it and woken by Scope, with no `Released` event.
    Released {
        /// The place.
        resource: Resource,
        /// The woken tasks.
        woken: Vec<TaskId>,
    },
    /// A program's task finished: halted by a `Stop` refusal, or shut down.
    Finished {
        /// The program.
        program: ProgramSlot,
        /// True after a `Stop` refusal.
        halted: bool,
        /// Its committed steps.
        steps: u64,
        /// Undelivered messages dropped with its mailbox.
        dropped: Vec<MessageId>,
        /// Held inputs (backlog or pending) dropped with it, oldest first.
        held: Vec<MessageId>,
    },
}

/// One entry of the run's trace, in the order the runtime made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEvent {
    /// Its number in the run, from 0. Dropped events keep their numbers, so a gap shows.
    pub seq: u64,
    /// Scope's logical clock when it happened (for events between epochs, the clock then).
    pub tick: Tick,
    /// What happened.
    pub kind: RuntimeEventKind,
}

impl RuntimeEvent {
    /// The step attempt, when this event is one.
    #[must_use]
    pub const fn step(&self) -> Option<&StepRecord> {
        match &self.kind {
            RuntimeEventKind::Step(s) => Some(s),
            _ => None,
        }
    }
}

/// One Scope scheduler event, as Scope's `Tracer` hooks report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedEvent {
    /// `on_round_begin`.
    Round(u64),
    /// `on_spawn`.
    Spawn(TaskId, String),
    /// `on_wake`.
    Wake(TaskId, WakeReason),
    /// `on_poll_begin`.
    Poll(u64, TaskId),
    /// `on_finish`.
    Finish(TaskId, TaskState),
    /// `on_defer_spawn`.
    DeferSpawn(TaskId, String),
    /// `on_cancel_request` (the time budget is left out: it is wall-clock).
    CancelRequest(TaskId, CancelReason, Option<u64>),
    /// `on_cancel_hard_stop`.
    HardStop(TaskId, CancelReason),
    /// `on_sleep_ticks`: a logical timer armed for this tick.
    SleepTicks(TaskId, Tick),
    /// `on_tick_timer`: a logical timer fired at this tick.
    TickTimer(TaskId, Tick),
}

/// The bounded event log both traces use: at most `retain` events, oldest dropped and counted.
#[derive(Debug)]
pub(crate) struct Retained<T> {
    pub(crate) events: VecDeque<T>,
    pub(crate) retain: usize,
    pub(crate) dropped: u64,
}

impl<T> Retained<T> {
    pub(crate) fn new(retain: usize) -> Self {
        Self {
            events: VecDeque::new(),
            retain,
            dropped: 0,
        }
    }

    pub(crate) fn push(&mut self, event: T) {
        if self.retain == 0 {
            self.dropped += 1;
            return;
        }
        if self.events.len() == self.retain {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }
}

pub(crate) struct SchedState {
    pub(crate) poll: u64,
    pub(crate) wakes: Vec<Option<WakeReason>>,
    pub(crate) events: Retained<SchedEvent>,
}

impl SchedState {
    pub(crate) fn new(retain: usize) -> Self {
        Self {
            poll: 0,
            wakes: Vec::new(),
            events: Retained::new(retain),
        }
    }

    pub(crate) fn wake_of(&self, id: TaskId) -> Option<WakeReason> {
        self.wakes.get(id.slot as usize).copied().flatten()
    }
}

/// The runtime's Scope tracer: records every hook into a buffer the runtime shares.
pub(crate) struct SchedRecorder(pub(crate) Rc<RefCell<SchedState>>);

impl SchedRecorder {
    fn push(&self, event: SchedEvent) {
        self.0.borrow_mut().events.push(event);
    }
}

impl Tracer for SchedRecorder {
    fn on_round_begin(&mut self, round: u64) {
        self.push(SchedEvent::Round(round));
    }
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.push(SchedEvent::Spawn(id, name.to_owned()));
    }
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        {
            let mut s = self.0.borrow_mut();
            let slot = id.slot as usize;
            if s.wakes.len() <= slot {
                s.wakes.resize(slot + 1, None);
            }
            s.wakes[slot] = Some(reason);
        }
        self.push(SchedEvent::Wake(id, reason));
    }
    fn on_poll_begin(&mut self, seq: u64, id: TaskId, _name: &str) {
        self.0.borrow_mut().poll = seq;
        self.push(SchedEvent::Poll(seq, id));
    }
    fn on_finish(&mut self, id: TaskId, _name: &str, state: TaskState) {
        self.push(SchedEvent::Finish(id, state));
    }
    fn on_defer_spawn(&mut self, parent: TaskId, child_name: &str) {
        self.push(SchedEvent::DeferSpawn(parent, child_name.to_owned()));
    }
    fn on_cancel_request(
        &mut self,
        id: TaskId,
        reason: CancelReason,
        tick_budget: Option<u64>,
        _time_budget: Option<Duration>,
    ) {
        self.push(SchedEvent::CancelRequest(id, reason, tick_budget));
    }
    fn on_cancel_hard_stop(&mut self, id: TaskId, reason: CancelReason) {
        self.push(SchedEvent::HardStop(id, reason));
    }
    fn on_sleep_ticks(&mut self, id: TaskId, at: Tick) {
        self.push(SchedEvent::SleepTicks(id, at));
    }
    fn on_tick_timer(&mut self, id: TaskId, at: Tick) {
        self.push(SchedEvent::TickTimer(id, at));
    }
}
