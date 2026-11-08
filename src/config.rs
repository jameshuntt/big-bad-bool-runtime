//! Every bound the runtime keeps, the admission policy, Scope's scheduler configuration and the
//! trace settings: one [`RuntimeConfig`], fixed at `build`.

use scope::SchedulerConfig;

/// Default most registered programs; past it `build` returns `BuildError::TooManyPrograms`.
pub const MAX_PROGRAMS: usize = 4096;
/// Default most routes in one runtime; past it `build` returns `BuildError::TooManyRoutes`.
pub const MAX_ROUTES: usize = 65_536;
/// Default most routes from one `(program, port)`; past it `build` returns `BuildError::FanOut`.
pub const MAX_FANOUT: usize = 64;
/// Default most undelivered messages per program: the capacity of its Scope bounded mailbox.
pub const MAX_MAILBOX: usize = 64;
/// Default largest retry backlog a program may have; a `Retry` policy asking for more is a
/// `BuildError::Backlog`.
pub const MAX_RETRY_BACKLOG: usize = 64;
/// Default most deliveries one step may make over all its sends; past it admission refuses it.
pub const MAX_DELIVERIES_PER_STEP: usize = 64;
/// Default most runtime effects one step may ask for; past it admission refuses the step.
pub const MAX_EFFECTS_PER_STEP: usize = 16;
/// Default most armed timers at once; past it admission refuses the step.
pub const MAX_TIMERS: usize = 256;
/// Default most live child activities at once; past it admission refuses the step.
pub const MAX_CHILDREN: usize = 256;
/// Default most committed program steps in one run; past it every input is refused
/// (`RuntimeRefusal::StepBudget`).
pub const MAX_STEPS: u64 = 1_000_000;
/// Default most runtime events kept in the trace; past it the oldest event is dropped and counted.
pub const MAX_TRACE_EVENTS: usize = 1_000_000;
/// Default scheduler rounds Scope gives a cancelled child before it hard-stops it.
pub const CHILD_CANCEL_TICKS: u64 = 2;

/// The runtime's bounds. Each has one deterministic result when it is reached; the field docs
/// name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// Registered programs. Past it: `BuildError::TooManyPrograms`.
    pub programs: usize,
    /// Routes in the runtime. Past it: `BuildError::TooManyRoutes`.
    pub routes: usize,
    /// Routes per `(program, port)`. Past it: `BuildError::FanOut`.
    pub fanout: usize,
    /// Undelivered messages per program, as senders see them (an input held by `Backpressure`
    /// takes one of these places). Full: admission refuses a step that would deliver
    /// (`RuntimeRefusal::MailboxFull`), an injection or a child tick is refused, a timer waits.
    pub mailbox: usize,
    /// The largest backlog a `Retry` policy may ask for. Past it: `BuildError::Backlog`.
    pub retry_backlog: usize,
    /// Deliveries per step, over all its sends. Past it: `RuntimeRefusal::FanOut`.
    pub deliveries_per_step: usize,
    /// Runtime effects per step. Past it: `RuntimeRefusal::EffectsPerStep`.
    pub effects_per_step: usize,
    /// Armed timers at once. Past it: `RuntimeRefusal::TooManyTimers`.
    pub timers: usize,
    /// Live child activities at once. Past it: `RuntimeRefusal::TooManyChildren`.
    pub children: usize,
    /// Committed program steps per run. Spent: `RuntimeRefusal::StepBudget` for every input.
    pub steps: u64,
    /// Runtime events kept in the trace (and, separately, Scope scheduler events). Past it the
    /// oldest is dropped and counted in `RunOutcome::trace_dropped` (`scheduler_dropped`); every
    /// event keeps its sequence number, so the gap shows.
    pub trace_events: usize,
    /// Rounds Scope gives a cancelled child before it hard-stops it.
    pub child_cancel_ticks: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            programs: MAX_PROGRAMS,
            routes: MAX_ROUTES,
            fanout: MAX_FANOUT,
            mailbox: MAX_MAILBOX,
            retry_backlog: MAX_RETRY_BACKLOG,
            deliveries_per_step: MAX_DELIVERIES_PER_STEP,
            effects_per_step: MAX_EFFECTS_PER_STEP,
            timers: MAX_TIMERS,
            children: MAX_CHILDREN,
            steps: MAX_STEPS,
            trace_events: MAX_TRACE_EVENTS,
            child_cancel_ticks: CHILD_CANCEL_TICKS,
        }
    }
}

/// What happens to an input whose prepared step admission refused.
///
/// Under every policy a refused step commits nothing and performs nothing. The policy only
/// decides what becomes of the input. A refusal is *final* when no later event of the run can
/// make the same step fit (`RuntimeRefusal::is_final`: too many effects or deliveries, no route,
/// a finished receiver, an unknown child, the step budget); a final refusal consumes the input
/// under every policy. The other refusals (a full mailbox, no free timer or child place, a child
/// key in use) wait for a place that the run frees by itself.
///
/// Held inputs (`Retry`, `Backpressure`) are attempted again at one fixed point: at the start of
/// the program's next poll, before any new mailbox message. Timer deliveries and child ticks are
/// mailbox messages, so held inputs go before them too. Each attempt is a new `prepare` from the
/// program's current state and a new admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdmissionPolicy {
    /// The input is consumed; nothing is committed. A host resends it in a later epoch.
    #[default]
    Consume,
    /// The input goes to the back of the program's retry backlog, a queue of at most `backlog`
    /// inputs. A full backlog drops the input with a `Disposition::BacklogFull` event. Each poll
    /// retries the backlog oldest first and stops at the first entry refused again, which keeps
    /// its place, so held inputs commit in the order they were refused. An entry refused
    /// `max_attempts` times in all is dropped (`Disposition::Exhausted`) and retrying goes on with
    /// the next one. After the backlog the poll takes at most one new mailbox message, so newer
    /// input can commit before older held input.
    Retry {
        /// Most inputs held at once.
        backlog: usize,
        /// Most refusals of one input before it is dropped; `u32::MAX` never drops.
        max_attempts: u32,
    },
    /// The input is held and the program takes no new mailbox message while it is held. The
    /// held input keeps a place of the program's mailbox, so the mailbox fills behind it and
    /// senders are refused `MailboxFull` by their own admission: pressure goes upstream. Input
    /// order is kept.
    Backpressure,
}

impl AdmissionPolicy {
    /// `Retry` with `backlog` places that never drops an input for its number of attempts.
    #[must_use]
    pub const fn retry(backlog: usize) -> Self {
        Self::Retry {
            backlog,
            max_attempts: u32::MAX,
        }
    }

    /// Stable lower-case name, for traces and reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Consume => "consume",
            Self::Retry { .. } => "retry",
            Self::Backpressure => "backpressure",
        }
    }
}

/// How much the run records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceLevel {
    /// Every runtime event with state digests, and Scope's scheduler events, up to
    /// `RuntimeLimits::trace_events` each.
    Full,
    /// Counts only: no event log, no digests, no Scope tracer. A runtime observer still sees
    /// every event.
    Off,
}

/// What a runtime is built with: its bounds, its admission policy, Scope's scheduler
/// configuration and the trace level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// The bounds.
    pub limits: RuntimeLimits,
    /// The admission policy of every program that does not name its own.
    pub policy: AdmissionPolicy,
    /// Scope's wake ordering, poll budget and default QoS, used as given.
    pub scheduler: SchedulerConfig,
    /// What is recorded.
    pub trace: TraceLevel,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            limits: RuntimeLimits::default(),
            policy: AdmissionPolicy::Consume,
            scheduler: SchedulerConfig::default(),
            trace: TraceLevel::Full,
        }
    }
}

impl RuntimeConfig {
    /// The default configuration under another Scope scheduler configuration.
    #[must_use]
    pub fn with_scheduler(scheduler: SchedulerConfig) -> Self {
        Self {
            scheduler,
            ..Self::default()
        }
    }

    /// This configuration with other bounds.
    #[must_use]
    pub const fn limits(mut self, limits: RuntimeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// This configuration with another admission policy.
    #[must_use]
    pub const fn policy(mut self, policy: AdmissionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// This configuration with another trace level.
    #[must_use]
    pub const fn trace(mut self, trace: TraceLevel) -> Self {
        self.trace = trace;
        self
    }
}
