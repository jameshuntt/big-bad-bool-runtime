//! PING-PONG: two rally programs return a hit to each other until each has counted four.
//!
//! `cargo run --example ping_pong`

use big_bad_bool_runtime::fixtures::{HIT, rally, rally_table, signal_message, verified};
use big_bad_bool_runtime::{
    ActionRecord, Delivery, Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig,
};

fn main() {
    let rally = verified(rally());
    let count = rally.spec().dimension_id("count").expect("count");
    let (a, b) = (ProgramId::from_name("ping"), ProgramId::from_name("pong"));
    let outcome = Runtime::builder(RuntimeConfig::default())
        .program(a, ProgramInstance::new(&rally, rally_table()))
        .program(b, ProgramInstance::new(&rally, rally_table()))
        .route(Route::new(a, Port(0), b))
        .route(Route::new(b, Port(0), a))
        .inject(a, signal_message(HIT))
        .build()
        .expect("within bounds")
        .run();

    println!("run {}", outcome.run_id);
    for event in &outcome.log {
        let Some(s) = event.step() else { continue };
        let contracts: Vec<&str> = s
            .results
            .iter()
            .flat_map(|r| r.committed.iter())
            .map(|c| rally.spec().contract(*c).expect("contract").name)
            .collect();
        let sent: Vec<String> = s
            .actions
            .iter()
            .flat_map(|a| match a {
                ActionRecord::Sent { deliveries, .. } => deliveries
                    .iter()
                    .map(|d| match d {
                        Delivery::Delivered { to, message } => format!("{message}->{to}"),
                        Delivery::Refused { message, reason } => {
                            format!("{message} refused {reason:?}")
                        }
                    })
                    .collect(),
                other => vec![format!("{other:?}")],
            })
            .collect();
        println!(
            "#{:<3} {} poll {:>2} {} took {} at {}: {} {:?} pre={} post={} {}",
            event.seq,
            event.tick,
            s.poll,
            s.program,
            s.message,
            s.position,
            contracts.join("+"),
            s.admission,
            &s.pre.0.as_str()[..8],
            &s.post.0.as_str()[..8],
            sent.join(" ")
        );
    }
    for p in &outcome.programs {
        println!(
            "{} ({}) final count={} steps={} digest={}",
            p.id,
            p.slot,
            p.state.word(count),
            p.steps,
            &p.digest.0.as_str()[..8]
        );
    }
    println!("external: {:?}", outcome.external);
}
