//! CANCELLATION: a commit starts a Scope child; a later commit cancels it through Scope.
//!
//! `cargo run --example cancellation`

#[allow(dead_code)]
#[path = "../tests/common/fixtures.rs"]
mod fixtures;

use big_bad_bool_runtime::{ProgramId, ProgramInstance, Runtime, RuntimeConfig, RuntimeEventKind};
use fixtures::{START, signal_message, supervisor, supervisor_table, verified};

fn main() {
    let p = verified(supervisor());
    let id = ProgramId::from_name("supervisor");
    let outcome = Runtime::builder(RuntimeConfig::default())
        .program(id, ProgramInstance::new(&p, supervisor_table()))
        .inject(id, signal_message(START))
        .build()
        .expect("within bounds")
        .run();
    for e in &outcome.log {
        match &e.kind {
            RuntimeEventKind::Step(s) => println!(
                "#{} {} step {} actions {:?}",
                e.seq, e.tick, s.message, s.actions
            ),
            RuntimeEventKind::ChildTick { key, delivery, .. } => {
                println!("#{} {} child {key:?} tick {delivery:?}", e.seq, e.tick)
            }
            RuntimeEventKind::ChildEnded { key, cancelled, .. } => println!(
                "#{} {} child {key:?} ended, cancelled: {cancelled}",
                e.seq, e.tick
            ),
            _ => {}
        }
    }
}
