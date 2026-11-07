//! FAN-OUT: one commit, eight receivers, admitted as one.
//!
//! `cargo run --example fan_out`

use big_bad_bool_runtime::fixtures::{
    START, broadcaster, broadcaster_table, signal_message, sink, verified,
};
use big_bad_bool_runtime::{
    ActionRecord, EffectTable, Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig,
};

fn main() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let hub = ProgramId::from_name("hub");
    let mut b = Runtime::builder(RuntimeConfig::default())
        .program(hub, ProgramInstance::new(&caster, broadcaster_table()));
    for n in 0..8 {
        let id = ProgramId::from_name(&format!("sink {n}"));
        b = b
            .program(id, ProgramInstance::new(&s, EffectTable::new()))
            .route(Route::new(hub, Port(0), id));
    }
    let outcome = b
        .inject(hub, signal_message(START))
        .build()
        .expect("within bounds")
        .run();
    let first = outcome.step_records().next().expect("the hub stepped");
    if let [ActionRecord::Sent { deliveries, .. }] = first.actions.as_slice() {
        for d in deliveries {
            println!("{d:?}");
        }
    }
    println!("steps {} refused {}", outcome.steps, outcome.refused);
}
