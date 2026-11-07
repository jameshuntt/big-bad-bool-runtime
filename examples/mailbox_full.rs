//! MAILBOX FULL under Consume: a prepared step whose send does not fit is refused before its
//! commit and consumed; resent in epoch 1, the same transition commits.
//!
//! `cargo run --example mailbox_full`

#[allow(dead_code)]
#[path = "../tests/common/fixtures.rs"]
mod fixtures;

use big_bad_bool_runtime::{
    EffectTable, Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig, RuntimeEventKind,
    RuntimeLimits,
};
use fixtures::{HIT, START, broadcaster, broadcaster_table, signal_message, sink, verified};
use scope::{SchedulerConfig, WakeOrdering};

fn main() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let (source, target) = (
        ProgramId::from_name("source"),
        ProgramId::from_name("target"),
    );
    let config = RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Fifo))
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
        .inject_in_epoch(1, source, signal_message(START))
        .build()
        .expect("within bounds")
        .run();
    for e in &outcome.log {
        match &e.kind {
            RuntimeEventKind::Epoch { epoch } => println!("epoch {epoch}"),
            RuntimeEventKind::Step(s) => println!(
                "  #{} {} {} {} {:?} {:?} pre={} post={}",
                e.seq,
                e.tick,
                s.program,
                s.message,
                s.admission,
                s.disposition,
                &s.pre.0.as_str()[..8],
                &s.post.0.as_str()[..8]
            ),
            _ => {}
        }
    }
}
