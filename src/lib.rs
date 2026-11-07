//! # big-bad-bool-runtime
//!
//! A deterministic runtime for many independent, verified big-bad-bool programs on one
//! [`scope`]. Each program is one Scope task that owns its `Stepper`; programs reach each other
//! only through messages on declared routes.
//!
//! ## Three layers, three jobs
//!
//! - **The program** (`big_bad_bool::program`) decides *semantic legality*: which contracts a
//!   step actualizes, what it writes, what effect words it emits, what it refuses. It holds no
//!   clock, no queue and no bound of the runtime's.
//! - **The runtime** (this crate) decides *admission and policy*: whether a prepared step's
//!   runtime effects fit every bound, and, when they do not, what becomes of the input
//!   ([`AdmissionPolicy`]).
//! - **Scope** decides *when*: which task runs in which round, under its wake ordering and poll
//!   budget, on its logical clock. Bounded mailboxes, logical timers, cancellation and task
//!   lifetimes are Scope's.
//!
//! ## One step: prepare, admit, commit
//!
//! 1. `Stepper::prepare` gives the step's results, effects and refusals with nothing changed;
//! 2. the program's [`EffectTable`] (data, resolved at `build`) maps the effect words to
//!    [`RuntimeEffect`]s, and admission checks all of them at once: free places in each
//!    receiver's bounded mailbox, deliveries per step, free timers, free children, effects per
//!    step, routes, child keys, the step budget;
//! 3. when everything fits, `Stepper::commit` applies exactly that preparation and the runtime
//!    acts (sends, Scope logical timers, Scope child tasks and their cancellation, external
//!    words, trace marks). When anything does not fit, nothing is committed and nothing is
//!    performed; the trace gets one [`RuntimeEvent`] with every reason and `pre == post`.
//!
//! ## Determinism
//!
//! A run is a function of the wake sequence, Scope's policy and the tick rule (one tick per
//! scheduler round): the same configuration and the same inputs give the same trace, the same
//! Scope events, the same external words and the same final states ([`RunOutcome::same_run`],
//! [`RunOutcome::diff`]). Nothing in the crate reads the wall clock, randomness or the
//! environment; real-time timers are outside replay and outside this crate.
//!
//! ```
//! use big_bad_bool_runtime::fixtures::{rally, rally_table, signal_message, verified, HIT};
//! use big_bad_bool_runtime::{Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig};
//!
//! let rally = verified(rally());
//! let (a, b) = (ProgramId::from_name("ping"), ProgramId::from_name("pong"));
//! let run = || {
//!     Runtime::builder(RuntimeConfig::default())
//!         .program(a, ProgramInstance::new(&rally, rally_table()))
//!         .program(b, ProgramInstance::new(&rally, rally_table()))
//!         .route(Route::new(a, Port(0), b))
//!         .route(Route::new(b, Port(0), a))
//!         .inject(a, signal_message(HIT))
//!         .build()
//!         .expect("within bounds")
//!         .run()
//! };
//! let outcome = run();
//! assert_eq!(outcome.steps, 9);
//! assert_eq!(outcome.external.len(), 1);
//! assert!(outcome.same_run(&run()));
//! ```
//!
//! ## Isolation at the type level
//!
//! A built runtime gives no access to a program's stepper or state; only the finished run hands
//! back final states:
//!
//! ```compile_fail,E0599
//! use big_bad_bool_runtime::fixtures::{rally, rally_table, verified};
//! use big_bad_bool_runtime::{ProgramId, ProgramInstance, Runtime, RuntimeConfig};
//!
//! let rally = verified(rally());
//! let runtime = Runtime::builder(RuntimeConfig::default())
//!     .program(ProgramId::from_name("a"), ProgramInstance::new(&rally, rally_table()))
//!     .build()
//!     .unwrap();
//! let _state = runtime.state(ProgramId::from_name("a"));
//! ```
//!
//! its registered programs (and their steppers) are private:
//!
//! ```compile_fail,E0616
//! use big_bad_bool_runtime::fixtures::{rally, rally_table, verified};
//! use big_bad_bool_runtime::{ProgramId, ProgramInstance, Runtime, RuntimeConfig};
//!
//! let rally = verified(rally());
//! let runtime = Runtime::builder(RuntimeConfig::default())
//!     .program(ProgramId::from_name("a"), ProgramInstance::new(&rally, rally_table()))
//!     .build()
//!     .unwrap();
//! let _programs = &runtime.instances;
//! ```
//!
//! and a program instance does not hand its program back for stepping:
//!
//! ```compile_fail,E0616
//! use big_bad_bool_runtime::fixtures::{rally, rally_table, verified};
//! use big_bad_bool_runtime::ProgramInstance;
//!
//! let rally = verified(rally());
//! let instance = ProgramInstance::new(&rally, rally_table());
//! let _table = instance.table;
//! ```
//!
//! An effect table holds data, not code: a closure is not an action.
//!
//! ```compile_fail,E0271
//! use big_bad_bool_runtime::EffectTable;
//!
//! let mut other_state = 0u128;
//! let _table = EffectTable::new().on("out", 1, [|| other_state += 1]);
//! ```

#![forbid(unsafe_code)]

#[cfg(feature = "bowl")]
mod bowl;
mod builder;
mod config;
mod effect;
pub mod fixtures;
mod held;
mod ids;
mod message;
mod observer;
mod outcome;
mod run;
mod trace;

#[cfg(feature = "bowl")]
pub use bowl::{BowlPhase, BowlRuntimeObserver, COMMITTED_BIT, HELD_BIT, REFUSED_BIT};
pub use builder::{BuildError, ProgramInstance, Route, Runtime, RuntimeBuilder};
pub use config::{
    AdmissionPolicy, CHILD_CANCEL_TICKS, MAX_CHILDREN, MAX_DELIVERIES_PER_STEP,
    MAX_EFFECTS_PER_STEP, MAX_FANOUT, MAX_MAILBOX, MAX_PROGRAMS, MAX_RETRY_BACKLOG, MAX_ROUTES,
    MAX_STEPS, MAX_TIMERS, MAX_TRACE_EVENTS, RuntimeConfig, RuntimeLimits, TraceLevel,
};
pub use effect::{ChildSpec, EffectRule, EffectTable, RuntimeEffect, TableError};
pub use ids::{
    ChildKey, MessageId, Origin, PROGRAM_NAMESPACE, Port, ProgramId, ProgramSlot, ROUTE_NAMESPACE,
    RUN_NAMESPACE, RouteId, RunId, TIMER_NAMESPACE, TimerId,
};
pub use message::RuntimeMessage;
pub use observer::{EventCollector, NoRuntimeObserver, RuntimeObserver};
pub use outcome::{ExternalRecord, PolicyCounts, ProgramFinal, RunDiff, RunOutcome};
pub use trace::{
    ActionRecord, Admission, Delivery, Disposition, InputSource, Resource, RuntimeEvent,
    RuntimeEventKind, RuntimeRefusal, SchedEvent, StateDigest, StepRecord, StepSummary,
    state_digest,
};
