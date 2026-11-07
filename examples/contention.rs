//! CONTENTION: the determinism table. Two workloads under 5 Scope policies x 3 poll budgets x 3
//! admission policies, 10 runs each: 16 rally pairs (no refusals), and 4 casters into one
//! target with two places (refusals under every policy).
//!
//! `cargo run --release --example contention`

use big_bad_bool::program::{NoObserver, VerifiedProgram};
#[allow(dead_code)]
#[path = "../tests/common/fixtures.rs"]
mod fixtures;

use big_bad_bool_runtime::{
    AdmissionPolicy, EffectTable, Port, ProgramId, ProgramInstance, Route, RunOutcome, Runtime,
    RuntimeConfig, RuntimeLimits,
};
use fixtures::{
    HIT, START, broadcaster, broadcaster_table, rally, rally_table, signal_message, sink, verified,
};
use scope::{SchedulerConfig, WakeOrdering};

const RUNS: usize = 10;

fn pairs(p: &VerifiedProgram, config: RuntimeConfig) -> RunOutcome<NoObserver> {
    let mut b = Runtime::builder(config);
    for pair in 0..16 {
        let x = ProgramId::from_name(&format!("pair {pair} x"));
        let y = ProgramId::from_name(&format!("pair {pair} y"));
        b = b
            .program(x, ProgramInstance::new(p, rally_table()))
            .program(y, ProgramInstance::new(p, rally_table()))
            .route(Route::new(x, Port(0), y))
            .route(Route::new(y, Port(0), x))
            .inject(x, signal_message(HIT));
    }
    b.build().expect("within bounds").run()
}

fn casters(
    c: &VerifiedProgram,
    s: &VerifiedProgram,
    config: RuntimeConfig,
) -> RunOutcome<NoObserver> {
    let target = ProgramId::from_name("target");
    let mut b =
        Runtime::builder(config).program(target, ProgramInstance::new(s, EffectTable::new()));
    for i in 0..4 {
        let id = ProgramId::from_name(&format!("caster {i}"));
        b = b
            .program(id, ProgramInstance::new(c, broadcaster_table()))
            .route(Route::new(id, Port(0), target))
            .inject(id, signal_message(START))
            .inject(id, signal_message(START));
    }
    b.build().expect("within bounds").run()
}

fn main() {
    let (r, c, s) = (verified(rally()), verified(broadcaster()), verified(sink()));
    let policies = [
        AdmissionPolicy::Consume,
        AdmissionPolicy::retry(4),
        AdmissionPolicy::Backpressure,
    ];
    println!(
        "| workload | scope policy | budget | admission | steps | refused | events | rounds | same over {RUNS} runs |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut all = true;
    for ordering in WakeOrdering::ALL {
        for budget in [usize::MAX, 1, 3] {
            let sched = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
            for policy in policies {
                let config = RuntimeConfig::with_scheduler(sched).policy(policy);
                let tight = config.limits(RuntimeLimits {
                    mailbox: 2,
                    ..RuntimeLimits::default()
                });
                let rows: [(&str, &dyn Fn() -> RunOutcome<NoObserver>); 2] = [
                    ("rally pairs", &|| pairs(&r, config)),
                    ("casters", &|| casters(&c, &s, tight)),
                ];
                for (name, run) in rows {
                    let first = run();
                    let same = (1..RUNS).all(|_| run().same_run(&first));
                    all &= same;
                    let budget = if budget == usize::MAX {
                        "all".to_owned()
                    } else {
                        budget.to_string()
                    };
                    println!(
                        "| {name} | {} | {budget} | {} | {} | {} | {} | {} | {same} |",
                        ordering.name(),
                        policy.name(),
                        first.steps,
                        first.refused,
                        first.events,
                        first.runs[0].rounds
                    );
                }
            }
        }
    }
    println!("every configuration repeated exactly: {all}");
}
