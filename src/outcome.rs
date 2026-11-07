//! What a run hands back, and how two runs are compared.

use big_bad_bool::program::ProjectionState;
use scope::{JoinError, RunReport};

use crate::ids::{MessageId, ProgramId, ProgramSlot, RunId, slot};
use crate::observer::NoRuntimeObserver;
use crate::trace::{RuntimeEvent, SchedEvent, StateDigest, StepRecord};

/// One effect word handed to the application.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalRecord {
    /// The program whose committed step asked for it.
    pub program: ProgramSlot,
    /// The input that step took.
    pub message: MessageId,
    /// The tag.
    pub tag: u32,
    /// The word.
    pub word: u128,
}

/// A program at the end of the run.
#[derive(Debug)]
pub struct ProgramFinal<O> {
    /// The program.
    pub id: ProgramId,
    /// Its slot in the run.
    pub slot: ProgramSlot,
    /// Its committed state.
    pub state: ProjectionState,
    /// The state's digest.
    pub digest: StateDigest,
    /// True when a `Stop` refusal halted it.
    pub halted: bool,
    /// Its committed steps.
    pub steps: u64,
    /// Its step attempts admission refused.
    pub refused: u64,
    /// Its step observer.
    pub observer: O,
}

/// What the admission policies did, over the whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PolicyCounts {
    /// Refused inputs consumed (`Disposition::Consumed`).
    pub consumed: u64,
    /// Inputs put in a retry backlog (`Disposition::Backlogged`).
    pub backlogged: u64,
    /// Backlog entries refused again and kept (`Disposition::Kept`).
    pub kept: u64,
    /// Inputs dropped by a full backlog (`Disposition::BacklogFull`).
    pub backlog_full: u64,
    /// Inputs dropped after `max_attempts` (`Disposition::Exhausted`).
    pub exhausted: u64,
    /// Attempts that left an input pending under `Backpressure` (`Disposition::Backpressured`).
    pub backpressured: u64,
    /// Attempts of held inputs (backlog or pending).
    pub retries: u64,
    /// Held inputs that committed.
    pub held_committed: u64,
}

/// Everything a run produced.
#[derive(Debug)]
pub struct RunOutcome<O, W = NoRuntimeObserver> {
    /// The run's id ([`crate::Runtime::run_id`]).
    pub run_id: RunId,
    /// The program ids, by slot.
    pub program_ids: Vec<ProgramId>,
    /// The retained runtime events, in sequence order (empty under `TraceLevel::Off`).
    pub log: Vec<RuntimeEvent>,
    /// Runtime events dropped from the front of the log by `RuntimeLimits::trace_events`.
    pub trace_dropped: u64,
    /// Runtime events made in all (retained plus dropped; with the trace off, those a runtime
    /// observer saw).
    pub events: u64,
    /// Scope's retained scheduler events (empty under `TraceLevel::Off`).
    pub scheduler: Vec<SchedEvent>,
    /// Scheduler events dropped from the front by `RuntimeLimits::trace_events`.
    pub scheduler_dropped: u64,
    /// Scope's report for each epoch, run until idle (cumulative).
    pub runs: Vec<RunReport>,
    /// Scope's report after shutdown (cumulative).
    pub shutdown: RunReport,
    /// Every program that finished, by slot.
    pub programs: Vec<ProgramFinal<O>>,
    /// Programs whose task ended without a result (a panic Scope isolated).
    pub lost: Vec<(ProgramId, JoinError)>,
    /// Every external effect, in order.
    pub external: Vec<ExternalRecord>,
    /// Committed program steps.
    pub steps: u64,
    /// Step attempts admission refused.
    pub refused: u64,
    /// Messages numbered.
    pub messages: u64,
    /// What the admission policies did.
    pub policy: PolicyCounts,
    /// The runtime observer, after the run.
    pub observer: W,
}

/// The first place two runs differ, in the order [`RunOutcome::diff`] compares them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunDiff {
    /// Different run ids (different topologies).
    RunId,
    /// Different program ids by slot.
    ProgramIds,
    /// The runtime events differ first at `index` of the retained logs (`None`: that log ended).
    Event {
        /// Index into both logs.
        index: usize,
        /// This run's event.
        left: Option<RuntimeEvent>,
        /// The other run's event.
        right: Option<RuntimeEvent>,
    },
    /// Scope's scheduler events differ first at `index`.
    Scheduler {
        /// Index into both logs.
        index: usize,
        /// This run's event.
        left: Option<SchedEvent>,
        /// The other run's event.
        right: Option<SchedEvent>,
    },
    /// The external effects differ first at `index`.
    External {
        /// Index into both lists.
        index: usize,
    },
    /// The final states differ first at this slot (`None`: different numbers of programs).
    FinalDigest {
        /// The first slot whose digest differs.
        slot: Option<ProgramSlot>,
    },
    /// A count differs.
    Count {
        /// Which (`steps`, `refused`, `messages`, `events`, `trace_dropped`,
        /// `scheduler_dropped`, `policy`).
        name: &'static str,
    },
}

fn first_difference<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Option<(usize, Option<T>, Option<T>)> {
    let len = a.len().max(b.len());
    (0..len).find_map(|i| {
        let (x, y) = (a.get(i), b.get(i));
        (x != y).then(|| (i, x.cloned(), y.cloned()))
    })
}

impl<O, W> RunOutcome<O, W> {
    /// The program `id` at the end.
    #[must_use]
    pub fn program(&self, id: ProgramId) -> Option<&ProgramFinal<O>> {
        self.programs.iter().find(|p| p.id == id)
    }

    /// The slot of `id`.
    #[must_use]
    pub fn slot(&self, id: ProgramId) -> Option<ProgramSlot> {
        self.program_ids.iter().position(|p| *p == id).map(slot)
    }

    /// Every program's final digest, by slot.
    #[must_use]
    pub fn final_digests(&self) -> Vec<(ProgramId, StateDigest)> {
        self.programs
            .iter()
            .map(|p| (p.id, p.digest.clone()))
            .collect()
    }

    /// Every step attempt in the retained log, in order.
    pub fn step_records(&self) -> impl Iterator<Item = &StepRecord> + '_ {
        self.log.iter().filter_map(RuntimeEvent::step)
    }

    /// Every step attempt of `id` in the retained log, in order.
    pub fn steps_of(&self, id: ProgramId) -> impl Iterator<Item = &StepRecord> + '_ {
        let s = self.slot(id);
        self.step_records().filter(move |r| Some(r.program) == s)
    }

    /// The first difference from `other`: ids, runtime events, Scope's scheduler events, external
    /// effects, final digests, then counts. `None` when they are the same run.
    #[must_use]
    pub fn diff<P, X>(&self, other: &RunOutcome<P, X>) -> Option<RunDiff> {
        if self.run_id != other.run_id {
            return Some(RunDiff::RunId);
        }
        if self.program_ids != other.program_ids {
            return Some(RunDiff::ProgramIds);
        }
        if let Some((index, left, right)) = first_difference(&self.log, &other.log) {
            return Some(RunDiff::Event { index, left, right });
        }
        if let Some((index, left, right)) = first_difference(&self.scheduler, &other.scheduler) {
            return Some(RunDiff::Scheduler { index, left, right });
        }
        if let Some((index, _, _)) = first_difference(&self.external, &other.external) {
            return Some(RunDiff::External { index });
        }
        let (a, b) = (self.final_digests(), other.final_digests());
        if a != b {
            let slot = (a.len() == b.len())
                .then(|| a.iter().zip(&b).position(|(x, y)| x != y).map(slot))
                .flatten();
            return Some(RunDiff::FinalDigest { slot });
        }
        let counts = [
            ("steps", self.steps == other.steps),
            ("refused", self.refused == other.refused),
            ("messages", self.messages == other.messages),
            ("events", self.events == other.events),
            ("trace_dropped", self.trace_dropped == other.trace_dropped),
            (
                "scheduler_dropped",
                self.scheduler_dropped == other.scheduler_dropped,
            ),
            ("policy", self.policy == other.policy),
        ];
        counts
            .into_iter()
            .find(|(_, same)| !same)
            .map(|(name, _)| RunDiff::Count { name })
    }

    /// Whether `other` is the same run: [`RunOutcome::diff`] finds nothing.
    #[must_use]
    pub fn same_run<P, X>(&self, other: &RunOutcome<P, X>) -> bool {
        self.diff(other).is_none()
    }
}
