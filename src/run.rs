//! The run: every program is one Scope task that owns its `Stepper`; everything they share is
//! Scope's environment `R` ([`Shared`]), which holds bookkeeping only: routes, accounting,
//! timers, children, waiters, counters. No `Stepper`, no `ProjectionState`.
//!
//! One step attempt has three phases: `Stepper::prepare` (nothing changes), admission (every
//! runtime effect of the step against every bound, see [`admit`]), then, only when all of it
//! fits, `Stepper::commit` and the actions. When anything does not fit, the preparation is
//! dropped: the state, the mailboxes, the timers and the children stay as they were, the trace
//! gets one `StepRecord` with every reason, and the program's `AdmissionPolicy` decides what
//! becomes of the input.
//!
//! One poll of a program task, in this order:
//!
//! 1. the run is shutting down: the task ends;
//! 2. held inputs: `Retry` retries its backlog oldest first and stops at the first entry refused
//!    again; `Backpressure` attempts its pending input;
//! 3. unless `Backpressure` still holds an input, at most one new mailbox message;
//! 4. when this poll freed a place of its own mailbox that Scope does not see (it committed or
//!    dropped the input `Backpressure` held) or took a message, the tasks on the runtime's list
//!    for that mailbox are woken; then the task yields (more mail), or waits.
//!
//! A held input waits for the place its refusal named ([`Resource`]). A mailbox place is Scope's:
//! the task registers in the receiver's bounded mailbox (`BoundedSender::wake_when_free`), a
//! receive holds the freed place for the oldest waiter and wakes it (`WakeReason::Capacity`), and
//! the woken task fills it with `try_send_from`. The task keeps the `CapacityWait` of each
//! mailbox it waits on and drops it when its refusals no longer name that mailbox, or when it
//! ends, which hands a held place to the next waiter. Two waits stay on a runtime list per
//! mailbox (`Shared::place_waiters`), because Scope cannot see the place they need: the place an
//! input `Backpressure` holds outside the mailbox (docs/MISSING_APIS.md, item 2), and a step that
//! needs more than one place in one mailbox. Timer and child places keep their runtime lists,
//! woken with `WakeReason::Manual`.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use big_bad_bool::program::{DimensionId, Observation, Observer, Stepper, VerifiedProgram};
use scope::{
    BoundedMailbox, BoundedSender, CancelEscalation, CancelReason, CapacityWait, Cx, JoinError,
    Scope, ScopedTask, TaskId, TaskPoll, Tick, TickSleepTask, TrySendError, WaitRegistration,
    WakeReason,
};

use crate::builder::{Instance, Runtime};
use crate::config::{AdmissionPolicy, RuntimeLimits, TraceLevel};
use crate::effect::{ChildSpec, ResolvedTable, RuntimeEffect};
use crate::held::{Backlog, Tried, settle_pending};
use crate::ids::{ChildKey, MessageId, Origin, Port, ProgramId, TimerId, slot};
use crate::message::RuntimeMessage;
use crate::observer::RuntimeObserver;
use crate::outcome::{ExternalRecord, PolicyCounts, ProgramFinal, RunOutcome};
use crate::trace::{
    ActionRecord, Admission, Delivery, Disposition, InputSource, Resource, Retained, RuntimeEvent,
    RuntimeEventKind, RuntimeRefusal, SchedRecorder, SchedState, StateDigest, StepRecord,
    StepSummary, state_digest,
};

/// The dimension id a message name resolves to when the receiver has no such dimension: the
/// receiver's stepper refuses it (`BBBC0108`, "no such dimension").
const NO_SUCH_DIMENSION: DimensionId = DimensionId(u16::MAX);

/// One message with its run identity.
pub(crate) struct Input {
    id: MessageId,
    origin: Origin,
    message: RuntimeMessage,
}

/// Every program's bounded mailbox sender, by slot, set once after every program task is
/// spawned; the run's journal (message counter, event counter, retained trace, observer); the
/// places `Backpressure` holds outside the mailboxes; and the shutdown flag. It lives outside
/// Scope's environment so the run can number and record injections between epochs. One thread.
struct Directory<'w> {
    senders: OnceCell<Vec<BoundedSender<Input>>>,
    record: bool,
    observing: bool,
    next_message: Cell<u64>,
    next_seq: Cell<u64>,
    log: RefCell<Retained<RuntimeEvent>>,
    held: Vec<Cell<usize>>,
    shutdown: Cell<bool>,
    observer: &'w RefCell<dyn RuntimeObserver + 'w>,
}

impl Directory<'_> {
    const fn emitting(&self) -> bool {
        self.record || self.observing
    }

    fn message_id(&self) -> MessageId {
        let id = self.next_message.get();
        self.next_message.set(id + 1);
        MessageId(id)
    }

    /// Numbers, shows and keeps one event. Nothing is built when nothing would see it.
    fn emit(&self, tick: Tick, kind: impl FnOnce() -> RuntimeEventKind) {
        if !self.emitting() {
            return;
        }
        let seq = self.next_seq.get();
        self.next_seq.set(seq + 1);
        let event = RuntimeEvent {
            seq,
            tick,
            kind: kind(),
        };
        if self.observing {
            self.observer.borrow_mut().on_event(&event);
        }
        if self.record {
            self.log.borrow_mut().push(event);
        }
    }

    fn sender(&self, slot: usize) -> &BoundedSender<Input> {
        &self.senders.get().expect("set before the first round")[slot]
    }

    /// Places sender `task` may fill in `slot`'s mailbox (`None`: a sender outside the run): the
    /// places nobody holds, and the one Scope holds for `task`, less the input `Backpressure` holds
    /// for `slot`. `None` when the mailbox is closed (the program finished).
    fn free(&self, slot: usize, task: Option<TaskId>) -> Option<usize> {
        let tx = self.sender(slot);
        let free = task.map_or_else(|| tx.free(), |t| tx.free_for(t));
        (!tx.is_closed()).then(|| free.saturating_sub(self.held[slot].get()))
    }
}

struct ChildSlot {
    task: Option<TaskId>,
    cancel_requested: bool,
}

/// Scope's environment for a run: bookkeeping only.
pub(crate) struct Shared {
    limits: RuntimeLimits,
    routes: Vec<Vec<(Port, Vec<usize>)>>,
    steps: u64,
    refused: u64,
    timers: usize,
    timers_armed: Vec<u32>,
    children: BTreeMap<(usize, ChildKey), ChildSlot>,
    external: Vec<ExternalRecord>,
    sched: Option<Rc<RefCell<SchedState>>>,
    /// Per mailbox, the tasks waiting for a place Scope cannot give them: the mailbox's
    /// `Backpressure` input holds it, or the step needs more than one.
    place_waiters: Vec<Vec<TaskId>>,
    timer_waiters: Vec<TaskId>,
    child_waiters: Vec<TaskId>,
    counts: PolicyCounts,
}

impl Shared {
    fn waiters(&mut self, resource: Resource) -> &mut Vec<TaskId> {
        match resource {
            Resource::Mailbox(s) => &mut self.place_waiters[s.0 as usize],
            Resource::Timers => &mut self.timer_waiters,
            Resource::Children => &mut self.child_waiters,
        }
    }

    /// `task` waits on the runtime list of `resource`, in the order tasks began waiting.
    fn wait_on_list(&mut self, task: TaskId, resource: Resource) {
        let list = self.waiters(resource);
        if !list.contains(&task) {
            list.push(task);
        }
    }
}

/// `resource` was freed: wakes every task on its runtime list, in the order they began waiting
/// (`WakeReason::Capacity` for a mailbox place, `WakeReason::Manual` for a timer or child place).
fn release<'env>(cx: &mut Cx<'_, 'env, Shared>, directory: &Directory<'_>, resource: Resource) {
    let woken = std::mem::take(cx.resources.waiters(resource));
    if woken.is_empty() {
        return;
    }
    let reason = match resource {
        Resource::Mailbox(_) => WakeReason::Capacity,
        Resource::Timers | Resource::Children => WakeReason::Manual,
    };
    for task in &woken {
        cx.wake_with(*task, reason);
    }
    directory.emit(cx.now_tick(), || RuntimeEventKind::Released {
        resource,
        woken,
    });
}

/// `task` waits for one place of `to`'s mailbox: Scope's capacity wake, whose registration comes
/// back to be kept, unless Scope finds a place free for the task that the runtime's bound does not
/// let it use (the input `Backpressure` holds for `to`), which puts the task on the runtime list.
fn wait_for_one_place<'env>(
    cx: &mut Cx<'_, 'env, Shared>,
    directory: &Directory<'_>,
    task: TaskId,
    to: usize,
) -> Option<CapacityWait<Input>> {
    match directory.sender(to).wake_when_free(task) {
        WaitRegistration::Registered(wait) => Some(wait),
        WaitRegistration::AlreadyRegistered | WaitRegistration::Closed => None,
        WaitRegistration::FreeNow => {
            cx.resources.wait_on_list(task, Resource::Mailbox(slot(to)));
            None
        }
    }
}

/// Gives up a registration in `to`'s mailbox. Its held place, if it had one, goes to the next
/// waiter or comes free, so the runtime list of that mailbox is woken too.
fn gave_up<'env>(
    cx: &mut Cx<'_, 'env, Shared>,
    directory: &Directory<'_>,
    wait: CapacityWait<Input>,
    to: usize,
) {
    drop(wait);
    release(cx, directory, Resource::Mailbox(slot(to)));
}

/// Sends one message already numbered into `to`'s bounded mailbox, as task `from` (which may
/// hold a place there) or from outside the run (`None`).
fn deliver_as(
    directory: &Directory<'_>,
    to: usize,
    input: Input,
    from: Option<TaskId>,
) -> Delivery {
    let to_slot = slot(to);
    let id = input.id;
    match directory.free(to, from) {
        None => {
            return Delivery::Refused {
                message: id,
                reason: RuntimeRefusal::TargetFinished { to: to_slot },
            };
        }
        Some(0) => {
            return Delivery::Refused {
                message: id,
                reason: RuntimeRefusal::MailboxFull {
                    to: to_slot,
                    needed: 1,
                    free: 0,
                },
            };
        }
        Some(_) => {}
    }
    let tx = directory.sender(to);
    let sent = match from {
        Some(task) => tx.try_send_from(input, task),
        None => tx.try_send(input),
    };
    match sent {
        Ok(()) => Delivery::Delivered {
            to: to_slot,
            message: id,
        },
        Err(TrySendError::Full(_)) => Delivery::Refused {
            message: id,
            reason: RuntimeRefusal::MailboxFull {
                to: to_slot,
                needed: 1,
                free: 0,
            },
        },
        Err(TrySendError::Closed(_)) => Delivery::Refused {
            message: id,
            reason: RuntimeRefusal::TargetFinished { to: to_slot },
        },
    }
}

// ---------------------------------------------------------------- admission

/// A runtime effect admitted, with what admission resolved for it.
enum Planned {
    Send {
        port: Port,
        targets: Vec<usize>,
        message: RuntimeMessage,
    },
    Timer {
        after: scope::TickDelta,
        message: RuntimeMessage,
    },
    StartChild {
        key: ChildKey,
        child: ChildSpec,
    },
    CancelChild {
        key: ChildKey,
    },
    External {
        tag: u32,
        word: u128,
    },
    Trace {
        tag: u32,
    },
}

/// Checks every runtime effect of one prepared step of program `me` against every bound, before
/// anything is done: effects per step, routes, deliveries per step, each receiver's free mailbox
/// places (and that it has not finished), free timer places, free child places and child keys.
/// Every refusal is collected, in that order, so the record says all that did not fit.
fn admit(
    shared: &Shared,
    directory: &Directory<'_>,
    me: usize,
    task: TaskId,
    effects: &[RuntimeEffect],
) -> Result<Vec<Planned>, Vec<RuntimeRefusal>> {
    let limits = shared.limits;
    let mut refusals = Vec::new();
    if effects.len() > limits.effects_per_step {
        refusals.push(RuntimeRefusal::EffectsPerStep {
            observed: effects.len(),
            permitted: limits.effects_per_step,
        });
    }
    let mut planned = Vec::with_capacity(effects.len());
    let mut deliveries = 0usize;
    let mut per_target: BTreeMap<usize, usize> = BTreeMap::new();
    let mut timers = 0usize;
    let mut starts: Vec<ChildKey> = Vec::new();
    for effect in effects {
        match effect {
            RuntimeEffect::Send { port, message } => {
                let Some((_, targets)) = shared.routes[me].iter().find(|(p, _)| p == port) else {
                    refusals.push(RuntimeRefusal::NoRoute { port: *port });
                    continue;
                };
                deliveries += targets.len();
                for t in targets {
                    *per_target.entry(*t).or_insert(0) += 1;
                }
                planned.push(Planned::Send {
                    port: *port,
                    targets: targets.clone(),
                    message: message.clone(),
                });
            }
            RuntimeEffect::Timer { after, message } => {
                timers += 1;
                planned.push(Planned::Timer {
                    after: *after,
                    message: message.clone(),
                });
            }
            RuntimeEffect::StartChild { key, child } => {
                if shared.children.contains_key(&(me, *key)) || starts.contains(key) {
                    refusals.push(RuntimeRefusal::ChildExists { key: *key });
                }
                starts.push(*key);
                planned.push(Planned::StartChild {
                    key: *key,
                    child: child.clone(),
                });
            }
            RuntimeEffect::CancelChild { key } => {
                if !shared.children.contains_key(&(me, *key)) {
                    refusals.push(RuntimeRefusal::ChildUnknown { key: *key });
                }
                planned.push(Planned::CancelChild { key: *key });
            }
            RuntimeEffect::External { tag, word } => planned.push(Planned::External {
                tag: *tag,
                word: *word,
            }),
            RuntimeEffect::Trace { tag } => planned.push(Planned::Trace { tag: *tag }),
        }
    }
    if deliveries > limits.deliveries_per_step {
        refusals.push(RuntimeRefusal::FanOut {
            observed: deliveries,
            permitted: limits.deliveries_per_step,
        });
    }
    for (to, needed) in &per_target {
        match directory.free(*to, Some(task)) {
            None => refusals.push(RuntimeRefusal::TargetFinished { to: slot(*to) }),
            Some(free) if free < *needed => refusals.push(RuntimeRefusal::MailboxFull {
                to: slot(*to),
                needed: *needed,
                free,
            }),
            Some(_) => {}
        }
    }
    let free_timers = limits.timers.saturating_sub(shared.timers);
    if timers > free_timers {
        refusals.push(RuntimeRefusal::TooManyTimers {
            needed: timers,
            free: free_timers,
        });
    }
    let free_children = limits.children.saturating_sub(shared.children.len());
    if starts.len() > free_children {
        refusals.push(RuntimeRefusal::TooManyChildren {
            needed: starts.len(),
            free: free_children,
        });
    }
    if refusals.is_empty() {
        Ok(planned)
    } else {
        Err(refusals)
    }
}

// ---------------------------------------------------------------- program task

/// The parts of a step record known once the attempt is over; the disposition comes after.
struct Draft {
    task: TaskId,
    poll: u64,
    wake: Option<WakeReason>,
    position: u32,
    pre: StateDigest,
    post: StateDigest,
    results: Vec<StepSummary>,
    effects: Vec<RuntimeEffect>,
    admission: Admission,
    actions: Vec<ActionRecord>,
}

/// One step attempt's result.
struct Attempt {
    tried: Tried,
    reasons: Vec<RuntimeRefusal>,
    draft: Option<Draft>,
}

struct ProgramTask<'p, 'd, 'w, O: Observer> {
    index: usize,
    id: ProgramId,
    program: &'p VerifiedProgram,
    stepper: Option<Stepper<'p, O>>,
    table: ResolvedTable,
    policy: AdmissionPolicy,
    inbox: BoundedMailbox<Input>,
    directory: &'d Directory<'w>,
    position: u32,
    steps: u64,
    refused: u64,
    backlog: Backlog<Input>,
    pending: Option<(Input, u32)>,
    /// The mailboxes this task waits on through Scope, by slot, each with its registration.
    waits: BTreeMap<usize, CapacityWait<Input>>,
    /// The mailboxes the refusals of this poll named.
    wanted: Vec<usize>,
    /// This poll freed a place of its own mailbox: it took a message it does not hold, or
    /// committed or dropped the input `Backpressure` held.
    freed: bool,
}

impl<'p, 'd, 'w, O: Observer> ProgramTask<'p, 'd, 'w, O> {
    /// The task waits for every place `reasons` name.
    fn wait_for<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>, reasons: &[RuntimeRefusal])
    where
        'd: 'env,
    {
        let task = cx.id();
        for r in reasons {
            match *r {
                RuntimeRefusal::MailboxFull { to, needed, .. } => {
                    let s = to.0 as usize;
                    if !self.wanted.contains(&s) {
                        self.wanted.push(s);
                    }
                    if needed > 1 {
                        // A held place is one place: give it up rather than keep it while
                        // waiting for more (two such senders could keep each other waiting).
                        if let Some(wait) = self.waits.remove(&s) {
                            gave_up(cx, self.directory, wait, s);
                        }
                        cx.resources.wait_on_list(task, Resource::Mailbox(to));
                    } else if let Some(wait) = wait_for_one_place(cx, self.directory, task, s) {
                        self.waits.insert(s, wait);
                    }
                }
                _ => {
                    if let Some(resource) = r.resource() {
                        cx.resources.wait_on_list(task, resource);
                    }
                }
            }
        }
    }

    /// After a poll: gives up every Scope wait this poll's refusals no longer name.
    fn settle_waits<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>)
    where
        'd: 'env,
    {
        let stale: Vec<usize> = self
            .waits
            .keys()
            .filter(|s| !self.wanted.contains(s))
            .copied()
            .collect();
        for s in stale {
            if let Some(wait) = self.waits.remove(&s) {
                gave_up(cx, self.directory, wait, s);
            }
        }
        self.wanted.clear();
    }

    fn stepper(&self) -> &Stepper<'p, O> {
        self.stepper
            .as_ref()
            .expect("the stepper lives until the task ends")
    }

    fn halted(&self) -> bool {
        self.stepper().is_halted()
    }

    fn digest(&self) -> StateDigest {
        state_digest(self.program, self.stepper().state())
    }

    /// Ends the task: drops what is left in the mailbox and what is held, gives up every place it
    /// waits on, records the program finished, closes its mailbox (Scope wakes every task waiting
    /// for a place in it; they find it closed) and wakes the runtime list of its mailbox.
    fn end<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>) -> TaskPoll
    where
        'd: 'env,
    {
        let mut dropped = Vec::new();
        while let Some(input) = self.inbox.try_recv() {
            dropped.push(input.id);
        }
        self.inbox.close();
        self.wanted.clear();
        for (s, wait) in std::mem::take(&mut self.waits) {
            gave_up(cx, self.directory, wait, s);
        }
        let mut held: Vec<MessageId> = self.backlog.drain().map(|i| i.id).collect();
        if let Some((input, _)) = self.pending.take() {
            held.push(input.id);
        }
        self.directory.held[self.index].set(0);
        let (program, halted, steps) = (slot(self.index), self.halted(), self.steps);
        self.directory
            .emit(cx.now_tick(), || RuntimeEventKind::Finished {
                program,
                halted,
                steps,
                dropped,
                held,
            });
        release(cx, self.directory, Resource::Mailbox(program));
        TaskPoll::Ready
    }

    fn finish(&mut self) -> ProgramFinal<O> {
        let stepper = self.stepper.take().expect("finished once");
        let state = stepper.state().clone();
        let digest = state_digest(self.program, &state);
        let halted = stepper.is_halted();
        ProgramFinal {
            id: self.id,
            slot: slot(self.index),
            state,
            digest,
            halted,
            steps: self.steps,
            refused: self.refused,
            observer: stepper.into_observer(),
        }
    }

    /// One attempt: prepare, admit, and commit then act only when everything fits.
    fn attempt<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>, input: &Input) -> Attempt
    where
        'd: 'env,
    {
        let emitting = self.directory.emitting();
        let pre = emitting.then(|| self.digest());
        let position = self.position;
        let task = cx.id();
        let (poll, wake) = cx.resources.sched.as_ref().map_or((0, None), |s| {
            let s = s.borrow();
            (s.poll, s.wake_of(task))
        });
        let draft = |pre: Option<StateDigest>,
                     post: Option<StateDigest>,
                     results: Vec<StepSummary>,
                     effects: Vec<RuntimeEffect>,
                     admission: Admission,
                     actions: Vec<ActionRecord>| {
            pre.map(|pre| Draft {
                task,
                poll,
                wake,
                position,
                pre,
                post: post.unwrap_or_else(|| unreachable!("digests are taken together")),
                results,
                effects,
                admission,
                actions,
            })
        };

        if cx.resources.steps >= cx.resources.limits.steps {
            let reasons = vec![RuntimeRefusal::StepBudget {
                permitted: cx.resources.limits.steps,
            }];
            cx.resources.refused += 1;
            self.refused += 1;
            let admission = Admission::Refused(reasons.clone());
            return Attempt {
                tried: Tried::Refused {
                    final_refusal: true,
                },
                reasons,
                draft: draft(
                    pre.clone(),
                    pre,
                    Vec::new(),
                    Vec::new(),
                    admission,
                    Vec::new(),
                ),
            };
        }

        let spec = self.program.spec();
        let observation = input
            .message
            .evidence()
            .iter()
            .fold(Observation::new(position), |o, (name, word)| {
                o.with(spec.dimension_id(name).unwrap_or(NO_SUCH_DIMENSION), *word)
            });

        // Phase 1: everything up to the write, on a copy. Nothing has changed yet.
        let prepared = self.stepper().prepare(observation);
        let effects: Vec<RuntimeEffect> = prepared
            .effects()
            .flat_map(|e| self.table.actions(e).iter().cloned())
            .collect();

        // Phase 2: the runtime's bounds, all of them, before anything is done.
        match admit(cx.resources, self.directory, self.index, task, &effects) {
            Err(reasons) => {
                // Refused: the preparation is dropped and nothing changes.
                let summaries = if emitting {
                    prepared.results().iter().map(StepSummary::of).collect()
                } else {
                    Vec::new()
                };
                drop(prepared);
                cx.resources.refused += 1;
                self.refused += 1;
                let final_refusal = reasons.iter().any(RuntimeRefusal::is_final);
                let admission = Admission::Refused(reasons.clone());
                Attempt {
                    tried: Tried::Refused { final_refusal },
                    reasons,
                    draft: draft(pre.clone(), pre, summaries, effects, admission, Vec::new()),
                }
            }
            Ok(plan) => {
                // Phase 3: commit exactly the prepared transition, then act.
                let results = self
                    .stepper
                    .as_mut()
                    .expect("live")
                    .commit(prepared)
                    .expect("prepared by this stepper at its current version");
                self.position = self.position.saturating_add(1);
                self.steps += 1;
                cx.resources.steps += 1;
                let actions = self.act(cx, input.id, plan);
                let post = emitting.then(|| self.digest());
                let summaries = if emitting {
                    results.iter().map(StepSummary::of).collect()
                } else {
                    Vec::new()
                };
                Attempt {
                    tried: Tried::Committed,
                    reasons: Vec::new(),
                    draft: draft(pre, post, summaries, effects, Admission::Admitted, actions),
                }
            }
        }
    }

    /// Records one attempt with what became of its input.
    fn record(
        &self,
        tick: Tick,
        message: MessageId,
        origin: Origin,
        source: InputSource,
        draft: Option<Draft>,
        disposition: Disposition,
    ) {
        let Some(d) = draft else { return };
        let (program, program_id) = (slot(self.index), self.id);
        self.directory.emit(tick, || {
            RuntimeEventKind::Step(StepRecord {
                program,
                program_id,
                task: d.task,
                poll: d.poll,
                wake: d.wake,
                message,
                origin,
                source,
                position: d.position,
                pre: d.pre,
                post: d.post,
                results: d.results,
                effects: d.effects,
                admission: d.admission,
                actions: d.actions,
                disposition,
            })
        });
    }

    fn count(shared: &mut Shared, source: InputSource, disposition: Disposition) {
        let c = &mut shared.counts;
        if !matches!(source, InputSource::Mailbox) {
            c.retries += 1;
            if disposition == Disposition::Committed {
                c.held_committed += 1;
            }
        }
        match disposition {
            Disposition::Committed => {}
            Disposition::Consumed { .. } => c.consumed += 1,
            Disposition::Backlogged { .. } => c.backlogged += 1,
            Disposition::Kept { .. } => c.kept += 1,
            Disposition::BacklogFull { .. } => c.backlog_full += 1,
            Disposition::Exhausted { .. } => c.exhausted += 1,
            Disposition::Backpressured { .. } => c.backpressured += 1,
        }
    }

    /// `Retry`: the backlog, oldest first, until an entry is refused again (it keeps its place).
    fn retry_backlog<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>, max_attempts: u32)
    where
        'd: 'env,
    {
        let mut backlog = std::mem::take(&mut self.backlog);
        while !self.halted() {
            let Some((input, attempts)) = backlog.front() else {
                break;
            };
            let (message, origin) = (input.id, input.origin);
            let source = InputSource::Backlog {
                attempt: attempts.saturating_add(1),
            };
            let a = self.attempt(cx, input);
            let (disposition, go_on) = backlog.settle(a.tried, max_attempts);
            if disposition.is_held() {
                self.wait_for(cx, &a.reasons);
            }
            Self::count(cx.resources, source, disposition);
            self.record(cx.now_tick(), message, origin, source, a.draft, disposition);
            if !go_on {
                break;
            }
        }
        self.backlog = backlog;
    }

    /// `Backpressure`: the pending input, if any.
    fn resume_pending<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>)
    where
        'd: 'env,
    {
        let Some((input, attempts)) = self.pending.take() else {
            return;
        };
        let source = InputSource::Pending {
            attempt: attempts.saturating_add(1),
        };
        let a = self.attempt(cx, &input);
        let (disposition, still) = settle_pending(a.tried, attempts);
        Self::count(cx.resources, source, disposition);
        self.record(
            cx.now_tick(),
            input.id,
            input.origin,
            source,
            a.draft,
            disposition,
        );
        match still {
            Some(n) => {
                self.wait_for(cx, &a.reasons);
                self.pending = Some((input, n));
            }
            None => {
                self.directory.held[self.index].set(0);
                self.freed = true;
            }
        }
    }

    /// A new input from the mailbox.
    fn take<'env>(&mut self, cx: &mut Cx<'_, 'env, Shared>, input: Input)
    where
        'd: 'env,
    {
        let (message, origin) = (input.id, input.origin);
        let a = self.attempt(cx, &input);
        let disposition = match a.tried {
            Tried::Committed => Disposition::Committed,
            Tried::Refused { final_refusal } => match self.policy {
                AdmissionPolicy::Consume => Disposition::Consumed { final_refusal },
                _ if final_refusal => Disposition::Consumed {
                    final_refusal: true,
                },
                AdmissionPolicy::Retry { .. } => self.backlog.offer(input),
                AdmissionPolicy::Backpressure => {
                    self.pending = Some((input, 1));
                    self.directory.held[self.index].set(1);
                    Disposition::Backpressured { attempts: 1 }
                }
            },
        };
        if disposition.is_held() {
            self.wait_for(cx, &a.reasons);
        }
        if self.pending.is_none() {
            self.freed = true;
        }
        Self::count(cx.resources, InputSource::Mailbox, disposition);
        self.record(
            cx.now_tick(),
            message,
            origin,
            InputSource::Mailbox,
            a.draft,
            disposition,
        );
    }

    /// Carries out an admitted step's actions, in order.
    fn act<'env>(
        &mut self,
        cx: &mut Cx<'_, 'env, Shared>,
        message_id: MessageId,
        plan: Vec<Planned>,
    ) -> Vec<ActionRecord>
    where
        'd: 'env,
    {
        let me = self.index;
        let mut actions = Vec::with_capacity(plan.len());
        for p in plan {
            let action = match p {
                Planned::Send {
                    port,
                    targets,
                    message,
                } => {
                    let origin = Origin::Program(slot(me));
                    let deliveries = targets
                        .into_iter()
                        .map(|to| {
                            let id = self.directory.message_id();
                            let input = Input {
                                id,
                                origin,
                                message: message.clone(),
                            };
                            deliver_as(self.directory, to, input, Some(cx.id()))
                        })
                        .collect();
                    ActionRecord::Sent { port, deliveries }
                }
                Planned::Timer { after, message } => {
                    let shared = &mut *cx.resources;
                    shared.timers += 1;
                    let ordinal = shared.timers_armed[me];
                    shared.timers_armed[me] += 1;
                    let timer = TimerId::of(self.id, ordinal);
                    let id = self.directory.message_id();
                    cx.spawn_later_named(
                        "timer",
                        TimerTask {
                            parent: me,
                            timer,
                            id,
                            message: Some(message),
                            sleep: TickSleepTask::new(after),
                            slept: false,
                            waiting: false,
                            wait: None,
                            directory: self.directory,
                        },
                    );
                    ActionRecord::TimerArmed {
                        timer,
                        message: id,
                        after: after.0,
                    }
                }
                Planned::StartChild { key, child } => {
                    cx.resources.children.insert(
                        (me, key),
                        ChildSlot {
                            task: None,
                            cancel_requested: false,
                        },
                    );
                    let ChildSpec::Ticker { ticks, message } = child;
                    cx.spawn_later_named(
                        "child",
                        TickerTask {
                            parent: me,
                            key,
                            remaining: ticks,
                            message,
                            registered: false,
                            directory: self.directory,
                        },
                    );
                    ActionRecord::ChildStarted { key }
                }
                Planned::CancelChild { key } => {
                    let task = cx.resources.children.get_mut(&(me, key)).and_then(|s| {
                        s.cancel_requested = true;
                        s.task
                    });
                    if let Some(task) = task {
                        let ticks = cx.resources.limits.child_cancel_ticks;
                        cx.cancel_escalate_with_reason(
                            task,
                            CancelEscalation::after_ticks(ticks),
                            CancelReason::UserRequest,
                        );
                    }
                    ActionRecord::ChildCancelRequested { key, task }
                }
                Planned::External { tag, word } => {
                    cx.resources.external.push(ExternalRecord {
                        program: slot(me),
                        message: message_id,
                        tag,
                        word,
                    });
                    ActionRecord::External { tag, word }
                }
                Planned::Trace { tag } => ActionRecord::Trace { tag },
            };
            actions.push(action);
        }
        actions
    }
}

impl<'env, 'p: 'env, 'd: 'env, 'w: 'env, O: Observer + 'env> ScopedTask<'env, Shared>
    for ProgramTask<'p, 'd, 'w, O>
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Shared>) -> TaskPoll {
        if self.directory.shutdown.get() {
            return self.end(cx);
        }
        self.freed = false;
        // Held inputs first, before any new mailbox message.
        match self.policy {
            AdmissionPolicy::Consume => {}
            AdmissionPolicy::Retry { max_attempts, .. } => self.retry_backlog(cx, max_attempts),
            AdmissionPolicy::Backpressure => self.resume_pending(cx),
        }
        // Then at most one new message, unless an input is still held back.
        if !self.halted() && self.pending.is_none() {
            if let Some(input) = self.inbox.try_recv() {
                self.take(cx, input);
            }
        }
        if self.freed {
            release(cx, self.directory, Resource::Mailbox(slot(self.index)));
        }
        self.settle_waits(cx);
        if self.halted() {
            self.end(cx)
        } else if self.pending.is_none() && !self.inbox.is_empty() {
            cx.yield_now()
        } else {
            TaskPoll::Pending
        }
    }
}

// ---------------------------------------------------------------- timers and children

struct TimerTask<'d, 'w> {
    parent: usize,
    timer: TimerId,
    id: MessageId,
    message: Option<RuntimeMessage>,
    sleep: TickSleepTask,
    slept: bool,
    waiting: bool,
    /// Its registration in the parent's mailbox while it waits for a place there.
    wait: Option<CapacityWait<Input>>,
    directory: &'d Directory<'w>,
}

impl<'env, 'd: 'env, 'w: 'env> ScopedTask<'env, Shared> for TimerTask<'d, 'w> {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Shared>) -> TaskPoll {
        // Scope's own logical sleep arms Scope's tick timer and completes once it fired.
        if !self.slept {
            if ScopedTask::<'env, Shared>::poll(&mut self.sleep, cx) == TaskPoll::Pending {
                return TaskPoll::Pending;
            }
            self.slept = true;
        }
        let program = slot(self.parent);
        let (timer, message) = (self.timer, self.id);
        let me = cx.id();
        if self.directory.free(self.parent, Some(me)) == Some(0) {
            // No place in the parent's mailbox: wait for one, keep the timer place.
            if let Some(wait) = wait_for_one_place(cx, self.directory, me, self.parent) {
                self.wait = Some(wait);
            }
            if !self.waiting {
                self.waiting = true;
                self.directory
                    .emit(cx.now_tick(), || RuntimeEventKind::TimerWaiting {
                        program,
                        timer,
                        message,
                    });
            }
            return TaskPoll::Pending;
        }
        cx.resources.timers -= 1;
        let input = Input {
            id: self.id,
            origin: Origin::Timer(program, self.timer),
            message: self.message.take().expect("fires once"),
        };
        let delivery = deliver_as(self.directory, self.parent, input, Some(me));
        // The delivery used the place held for it, if one was: the registration is spent.
        self.wait = None;
        self.directory
            .emit(cx.now_tick(), || RuntimeEventKind::TimerFired {
                program,
                timer,
                delivery,
            });
        release(cx, self.directory, Resource::Timers);
        TaskPoll::Ready
    }
}

struct TickerTask<'d, 'w> {
    parent: usize,
    key: ChildKey,
    remaining: u32,
    message: RuntimeMessage,
    registered: bool,
    directory: &'d Directory<'w>,
}

impl TickerTask<'_, '_> {
    fn end<'env>(&self, cx: &mut Cx<'_, 'env, Shared>, cancelled: bool) -> TaskPoll {
        cx.resources.children.remove(&(self.parent, self.key));
        let (program, key) = (slot(self.parent), self.key);
        self.directory
            .emit(cx.now_tick(), || RuntimeEventKind::ChildEnded {
                program,
                key,
                cancelled,
            });
        release(cx, self.directory, Resource::Children);
        TaskPoll::Ready
    }
}

impl<'env, 'd: 'env, 'w: 'env> ScopedTask<'env, Shared> for TickerTask<'d, 'w> {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Shared>) -> TaskPoll {
        let id = cx.id();
        let flagged = cx.is_cancelled();
        let early = if self.registered {
            false
        } else {
            // `Cx::spawn_later_named` returns no id, so the child registers its own.
            self.registered = true;
            let s = cx
                .resources
                .children
                .get_mut(&(self.parent, self.key))
                .expect("registered by the parent");
            s.task = Some(id);
            s.cancel_requested
        };
        if flagged || early {
            return self.end(cx, true);
        }
        if self.remaining == 0 || self.directory.free(self.parent, None).is_none() {
            return self.end(cx, false);
        }
        self.remaining -= 1;
        let program = slot(self.parent);
        let input = Input {
            id: self.directory.message_id(),
            origin: Origin::Child(program, self.key),
            message: self.message.clone(),
        };
        let delivery = deliver_as(self.directory, self.parent, input, None);
        let key = self.key;
        self.directory
            .emit(cx.now_tick(), || RuntimeEventKind::ChildTick {
                program,
                key,
                delivery,
            });
        cx.yield_now()
    }
}

// ---------------------------------------------------------------- the run

impl<'p, O: Observer, W: RuntimeObserver> Runtime<'p, O, W> {
    /// Runs every program on one Scope and returns what the run did.
    ///
    /// Each epoch sends its injections, then runs Scope until nothing is runnable and no timer
    /// waits. After the last epoch the run sets its shutdown flag and wakes every program
    /// (`Scope::waker_for(.., WakeReason::Shutdown)`); Scope runs again, each task ends (dropping
    /// what it holds, recorded) and hands back its final state through Scope's join handle.
    /// Every bound reached during the run is a recorded, deterministic event, so the run has no
    /// error of its own: a program task that panicked is in `RunOutcome::lost`.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn run(self) -> RunOutcome<O, W> {
        let Runtime {
            config,
            run_id,
            ids,
            instances,
            routes,
            injections,
            observer,
            ..
        } = self;
        let n = ids.len();
        let limits = config.limits;
        let record = config.trace == TraceLevel::Full;
        let sched = record.then(|| Rc::new(RefCell::new(SchedState::new(limits.trace_events))));
        let mut shared = Shared {
            limits,
            routes,
            steps: 0,
            refused: 0,
            timers: 0,
            timers_armed: vec![0; n],
            children: BTreeMap::new(),
            external: Vec::new(),
            sched: sched.clone(),
            place_waiters: vec![Vec::new(); n],
            timer_waiters: Vec::new(),
            child_waiters: Vec::new(),
            counts: PolicyCounts::default(),
        };
        let last_epoch = injections.last().map_or(0, |(e, _, _)| *e);
        let capacity = limits.mailbox;
        let observer = RefCell::new(observer);
        let observing = observer.borrow().observes();

        let directory = Directory {
            senders: OnceCell::new(),
            record,
            observing,
            next_message: Cell::new(0),
            next_seq: Cell::new(0),
            log: RefCell::new(Retained::new(limits.trace_events)),
            held: (0..n).map(|_| Cell::new(0)).collect(),
            shutdown: Cell::new(false),
            observer: &observer,
        };
        let mut handles = Vec::with_capacity(n);
        let mut runs = Vec::new();
        let shutdown = {
            let mut scope = Scope::with_config(&mut shared, config.scheduler);
            if let Some(s) = &sched {
                scope.set_tracer(Box::new(SchedRecorder(s.clone())));
            }
            let mut senders = Vec::with_capacity(n);
            for (index, instance) in instances.into_iter().enumerate() {
                let Instance {
                    program,
                    stepper,
                    table,
                    policy,
                } = instance;
                let id = ids[index];
                let directory = &directory;
                let backlog = match policy {
                    AdmissionPolicy::Retry { backlog, .. } => Backlog::new(backlog),
                    _ => Backlog::new(0),
                };
                let (handle, tx) = scope.spawn_with_bounded_mailbox_result(
                    format!("program {index}"),
                    capacity,
                    move |inbox| ProgramTask {
                        index,
                        id,
                        program,
                        stepper: Some(stepper),
                        table,
                        policy,
                        inbox,
                        directory,
                        position: 0,
                        steps: 0,
                        refused: 0,
                        backlog,
                        pending: None,
                        waits: BTreeMap::new(),
                        wanted: Vec::new(),
                        freed: false,
                    },
                    |task: &mut ProgramTask<'p, '_, '_, O>, _shared: &mut Shared| task.finish(),
                );
                handles.push((id, handle));
                senders.push(tx);
            }
            let _ = directory.senders.set(senders);
            let mut pending = injections.into_iter().peekable();
            for epoch in 0..=last_epoch {
                let mut sent = Vec::new();
                while let Some((_, to, message)) = pending.next_if(|(e, _, _)| *e == epoch) {
                    sent.push((to, message));
                }
                directory.emit(scope.now_tick(), || RuntimeEventKind::Epoch { epoch });
                for (to, message) in sent {
                    let input = Input {
                        id: directory.message_id(),
                        origin: Origin::External,
                        message,
                    };
                    let delivery = deliver_as(&directory, to, input, None);
                    directory.emit(scope.now_tick(), || RuntimeEventKind::Injected { delivery });
                }
                runs.push(scope.run());
            }
            directory.shutdown.set(true);
            for index in 0..n {
                scope
                    .waker_for(directory.sender(index).target(), WakeReason::Shutdown)
                    .wake();
            }
            scope.run()
        };

        let mut finals = Vec::with_capacity(n);
        let mut lost = Vec::new();
        for (id, handle) in handles {
            match handle.take() {
                Some(Ok(f)) => finals.push(f),
                Some(Err(e)) => lost.push((id, e)),
                None => lost.push((id, JoinError::Panicked)),
            }
        }
        let (scheduler, scheduler_dropped) = sched.map_or((Vec::new(), 0), |s| {
            let mut s = s.borrow_mut();
            (
                std::mem::take(&mut s.events.events).into(),
                s.events.dropped,
            )
        });
        let log = directory.log.replace(Retained::new(0));
        let (events, messages) = (directory.next_seq.get(), directory.next_message.get());
        drop(directory);
        RunOutcome {
            run_id,
            program_ids: ids,
            log: log.events.into(),
            trace_dropped: log.dropped,
            events,
            scheduler,
            scheduler_dropped,
            runs,
            shutdown,
            programs: finals,
            lost,
            external: shared.external,
            steps: shared.steps,
            refused: shared.refused,
            messages,
            policy: shared.counts,
            observer: observer.into_inner(),
        }
    }
}
