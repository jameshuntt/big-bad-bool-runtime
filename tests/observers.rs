//! Observer isolation: a runtime observer sees every event and changes nothing.

mod common;

use big_bad_bool_runtime::fixtures::{HIT, broadcaster, rally, rally_table, signal_message, sink, verified};
use big_bad_bool_runtime::{
    AdmissionPolicy, EventCollector, Port, ProgramInstance, Route, Runtime, RuntimeConfig,
    RuntimeLimits, TraceLevel,
};

use common::{casters, id, rally_pairs};

#[test]
fn a_runtime_observer_sees_exactly_the_trace_and_changes_nothing() {
    let p = verified(rally());
    let plain = rally_pairs(&p, 2, RuntimeConfig::default());
    let build = |config: RuntimeConfig| {
        let mut b = Runtime::builder(config).observer(EventCollector::default());
        for pair in 0..2 {
            let (x, y) = common::pair_ids(pair);
            b = b
                .program(x, ProgramInstance::new(&p, rally_table()))
                .program(y, ProgramInstance::new(&p, rally_table()))
                .route(Route::new(x, Port(0), y))
                .route(Route::new(y, Port(0), x))
                .inject(x, signal_message(HIT));
        }
        b.build().unwrap().run()
    };
    let watched = build(RuntimeConfig::default());
    assert!(watched.same_run(&plain), "{:?}", watched.diff(&plain));
    assert_eq!(watched.observer.events, watched.log);
    // With the trace off the observer still sees every event, and the run is unchanged.
    let off = build(RuntimeConfig::default().trace(TraceLevel::Off));
    assert!(off.log.is_empty());
    assert_eq!(off.observer.events.len() as u64, plain.events);
    assert_eq!(off.final_digests(), plain.final_digests());
    let mut seen = off.observer.events.clone();
    let mut traced = plain.log.clone();
    // Scope's poll numbers and wakes come from the Scope tracer, which is off: compare the rest.
    for e in seen.iter_mut().chain(traced.iter_mut()) {
        if let big_bad_bool_runtime::RuntimeEventKind::Step(s) = &mut e.kind {
            s.poll = 0;
            s.wake = None;
        }
    }
    assert_eq!(seen, traced);
}

#[test]
fn an_observed_refusing_run_is_the_same_run() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    for policy in common::POLICIES {
        let config = RuntimeConfig::default().policy(policy).limits(RuntimeLimits {
            mailbox: 2,
            ..RuntimeLimits::default()
        });
        let plain = casters(&caster, &s, config);
        let target = id("target");
        let mut b = Runtime::builder(config)
            .observer(EventCollector::default())
            .program(target, ProgramInstance::new(&s, big_bad_bool_runtime::EffectTable::new()));
        for i in 0..4 {
            let c = id(&format!("caster {i}"));
            b = b
                .program(c, ProgramInstance::new(&caster, big_bad_bool_runtime::fixtures::broadcaster_table()))
                .route(Route::new(c, Port(0), target))
                .inject(c, signal_message(big_bad_bool_runtime::fixtures::START))
                .inject(c, signal_message(big_bad_bool_runtime::fixtures::START));
        }
        let watched = b.build().unwrap().run();
        assert!(plain.refused > 0);
        assert!(watched.same_run(&plain), "{}", policy.name());
        assert_eq!(watched.observer.events, watched.log);
        let _ = AdmissionPolicy::Consume;
    }
}
