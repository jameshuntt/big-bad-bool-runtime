//! LOGICAL TIMER: a commit arms a Scope logical timer; ten more runs are the same run.
//!
//! `cargo run --example logical_timer`

use big_bad_bool_runtime::fixtures::{START, alarm, alarm_table, signal_message, verified};
use big_bad_bool_runtime::{ProgramId, ProgramInstance, Runtime, RuntimeConfig, RuntimeEventKind};

fn main() {
    let p = verified(alarm());
    let id = ProgramId::from_name("alarm");
    let run = || {
        Runtime::builder(RuntimeConfig::default())
            .program(id, ProgramInstance::new(&p, alarm_table()))
            .inject(id, signal_message(START))
            .build()
            .expect("within bounds")
            .run()
    };
    let first = run();
    for e in &first.log {
        match &e.kind {
            RuntimeEventKind::Step(s) => println!(
                "#{} {} step {} {:?} {:?}",
                e.seq, e.tick, s.message, s.admission, s.actions
            ),
            RuntimeEventKind::TimerFired {
                timer, delivery, ..
            } => println!("#{} {} timer {timer} fired: {delivery:?}", e.seq, e.tick),
            _ => {}
        }
    }
    let same = (0..10).all(|_| run().same_run(&first));
    println!("ten more runs are the same run: {same}");
}
