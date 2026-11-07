//! POLICIES: one refused send under Consume, Retry and Backpressure. The source is polled first
//! (FIFO) and finds the target's mailbox full.
//!
//! `cargo run --example policies`

#[allow(dead_code)]
#[path = "../tests/common/fixtures.rs"]
mod fixtures;

use big_bad_bool_runtime::{
    AdmissionPolicy, EffectTable, Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig,
    RuntimeEventKind, RuntimeLimits,
};
use fixtures::{HIT, START, broadcaster, broadcaster_table, signal_message, sink, verified};
use scope::{SchedulerConfig, WakeOrdering};

fn main() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let (source, target) = (
        ProgramId::from_name("source"),
        ProgramId::from_name("target"),
    );
    for policy in [
        AdmissionPolicy::Consume,
        AdmissionPolicy::retry(4),
        AdmissionPolicy::Backpressure,
    ] {
        let config =
            RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Fifo))
                .policy(policy)
                .limits(RuntimeLimits {
                    mailbox: 2,
                    ..RuntimeLimits::default()
                });
        let outcome = Runtime::builder(config)
            .program(source, ProgramInstance::new(&caster, broadcaster_table()))
            .program(target, ProgramInstance::new(&s, EffectTable::new()))
            .route(Route::new(source, Port(0), target))
            .inject(target, signal_message(HIT))
            .inject(target, signal_message(HIT))
            .inject(source, signal_message(START))
            .inject(source, signal_message(START))
            .build()
            .expect("within bounds")
            .run();
        println!("{}:", policy.name());
        for e in &outcome.log {
            match &e.kind {
                RuntimeEventKind::Step(s) if s.program.0 == 0 => println!(
                    "  #{:<3} {} {} {:?} {:?} {:?}",
                    e.seq, e.tick, s.message, s.source, s.admission, s.disposition
                ),
                RuntimeEventKind::Released { resource, woken } => {
                    println!(
                        "  #{:<3} {} released {resource:?}, woke {woken:?}",
                        e.seq, e.tick
                    );
                }
                _ => {}
            }
        }
        println!(
            "  steps {} refused {} {:?}",
            outcome.steps, outcome.refused, outcome.policy
        );
    }
}
