//! Shared helpers: standalone replays and runtime shapes the tests reuse.

#![allow(dead_code)]

use big_bad_bool::program::{NoObserver, Observation, Stepper, VerifiedProgram};
use big_bad_bool_runtime::fixtures::{HIT, START, broadcaster_table, rally_table, signal_message};
use big_bad_bool_runtime::{
    Admission, AdmissionPolicy, EffectTable, Port, ProgramId, ProgramInstance, Route, RunOutcome,
    Runtime, RuntimeConfig, RuntimeLimits, RuntimeMessage, RuntimeRefusal, StateDigest, StepRecord,
    state_digest,
};
use scope::{SchedulerConfig, WakeOrdering};

/// Steps `program` alone, with no runtime, over `inputs` at positions 0, 1, ...
pub fn standalone(program: &VerifiedProgram, inputs: &[RuntimeMessage]) -> StateDigest {
    let spec = program.spec();
    let mut stepper = Stepper::new(program, NoObserver);
    for (position, m) in (0..).zip(inputs) {
        let o = m
            .evidence()
            .iter()
            .fold(Observation::new(position), |o, (n, w)| {
                o.with(spec.dimension_id(n).expect("known"), *w)
            });
        let _ = stepper.push(o);
    }
    state_digest(program, stepper.state())
}

/// The start state's digest.
pub fn start_digest(program: &VerifiedProgram) -> StateDigest {
    state_digest(program, Stepper::new(program, NoObserver).state())
}

pub fn id(name: &str) -> ProgramId {
    ProgramId::from_name(name)
}

/// The ids of rally pair `pair`.
pub fn pair_ids(pair: u32) -> (ProgramId, ProgramId) {
    (id(&format!("pair {pair} x")), id(&format!("pair {pair} y")))
}

/// `pairs` rally pairs, each served once.
pub fn rally_pairs(
    program: &VerifiedProgram,
    pairs: u32,
    config: RuntimeConfig,
) -> RunOutcome<NoObserver> {
    let mut b = Runtime::builder(config);
    for pair in 0..pairs {
        let (x, y) = pair_ids(pair);
        b = b
            .program(x, ProgramInstance::new(program, rally_table()))
            .program(y, ProgramInstance::new(program, rally_table()))
            .route(Route::new(x, Port(0), y))
            .route(Route::new(y, Port(0), x))
            .inject(x, signal_message(HIT));
    }
    b.build().expect("within bounds").run()
}

/// Four casters sending twice each into one target with two places: a run that refuses.
pub fn casters(
    caster: &VerifiedProgram,
    sink: &VerifiedProgram,
    config: RuntimeConfig,
) -> RunOutcome<NoObserver> {
    let target = id("target");
    let mut b =
        Runtime::builder(config).program(target, ProgramInstance::new(sink, EffectTable::new()));
    for i in 0..4 {
        let c = id(&format!("caster {i}"));
        b = b
            .program(c, ProgramInstance::new(caster, broadcaster_table()))
            .route(Route::new(c, Port(0), target))
            .inject(c, signal_message(START))
            .inject(c, signal_message(START));
    }
    b.build().unwrap().run()
}

pub fn scheduler(config: SchedulerConfig) -> RuntimeConfig {
    RuntimeConfig::with_scheduler(config)
}

pub fn fifo(f: impl FnOnce(&mut RuntimeLimits)) -> RuntimeConfig {
    let mut l = RuntimeLimits::default();
    f(&mut l);
    RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Fifo)).limits(l)
}

pub fn steps<O, W>(outcome: &RunOutcome<O, W>) -> Vec<&StepRecord> {
    outcome.step_records().collect()
}

pub fn refused(r: &StepRecord) -> &[RuntimeRefusal] {
    match &r.admission {
        Admission::Refused(reasons) => reasons,
        Admission::Admitted => &[],
    }
}

pub const BUDGETS: [usize; 3] = [usize::MAX, 1, 3];

pub const POLICIES: [AdmissionPolicy; 3] = [
    AdmissionPolicy::Consume,
    AdmissionPolicy::Retry {
        backlog: 4,
        max_attempts: u32::MAX,
    },
    AdmissionPolicy::Backpressure,
];
