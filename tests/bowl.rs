//! The optional ob-serve-a-bowl observers: runtime events and program step events, observers
//! only. A refused step never looks like a commit.

#![cfg(feature = "bowl")]

mod common;

use big_bad_bool::program::ob_serve_a_bowl::{
    ObservableWakeRuntime, ObserverConfig, WakeProfileCode,
};
use big_bad_bool::program::{BowlObserver, NoObserver};
use big_bad_bool_runtime::{
    AdmissionPolicy, BowlPhase, BowlRuntimeObserver, COMMITTED_BIT, EffectTable, Port,
    ProgramInstance, REFUSED_BIT, Route, Runtime, RuntimeConfig, RuntimeLimits,
};
use common::fixtures::{
    HIT, START, broadcaster, broadcaster_table, rally, rally_table, signal_message, sink, verified,
};
use scope::{SchedulerConfig, WakeOrdering};

use common::id;

fn bowl_runtime() -> ObservableWakeRuntime {
    let mut runtime = ObservableWakeRuntime::new();
    runtime.register(ObserverConfig::new("trace", WakeProfileCode::InlineQueue));
    runtime
}

#[test]
fn bowl_on_and_off_is_the_same_run() {
    let p = verified(rally());
    let (a, b) = common::pair_ids(0);
    let plain = common::rally_pairs(&p, 1, RuntimeConfig::default());
    let observed = Runtime::builder(RuntimeConfig::default())
        .observer(BowlRuntimeObserver::new(bowl_runtime()))
        .program(
            a,
            ProgramInstance::new(&p, rally_table()).observer(BowlObserver::new(bowl_runtime())),
        )
        .program(
            b,
            ProgramInstance::new(&p, rally_table()).observer(BowlObserver::new(bowl_runtime())),
        )
        .route(Route::new(a, Port(0), b))
        .route(Route::new(b, Port(0), a))
        .inject(a, signal_message(HIT))
        .build()
        .unwrap()
        .run();
    assert!(observed.same_run(&plain), "{:?}", observed.diff(&plain));
    let _: &NoObserver = &plain.programs[0].observer;
    for mut f in observed.programs {
        let published = f.observer.published();
        let delivered = f.observer.runtime_mut().drain_inline();
        // Per committed step: Observed, Framed, Candidate, Authorized, Committed, Emitted.
        assert_eq!(published, f.steps as usize * 6, "{}", f.id);
        assert_eq!(delivered.len(), published);
    }
    let mut runtime_observer = observed.observer;
    let delivered = runtime_observer.runtime_mut().drain_inline();
    assert_eq!(delivered.len(), runtime_observer.published());
    let committed = delivered
        .iter()
        .filter(|e| e.key.phase() == BowlPhase::Committed as u64)
        .count();
    assert_eq!(committed as u64, plain.steps);
}

#[test]
fn a_refused_step_never_looks_like_a_commit() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let (source, target) = (id("source"), id("target"));
    for policy in common::POLICIES {
        let config =
            RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Fifo))
                .policy(policy)
                .limits(RuntimeLimits {
                    mailbox: 1,
                    ..RuntimeLimits::default()
                });
        let outcome = Runtime::builder(config)
            .observer(BowlRuntimeObserver::new(bowl_runtime()))
            .program(
                source,
                ProgramInstance::new(&caster, broadcaster_table())
                    .observer(BowlObserver::new(bowl_runtime())),
            )
            .program(
                target,
                ProgramInstance::new(&s, EffectTable::new())
                    .observer(BowlObserver::new(bowl_runtime())),
            )
            .route(Route::new(source, Port(0), target))
            .inject(target, signal_message(HIT))
            .inject(source, signal_message(START))
            .build()
            .unwrap()
            .run();
        let refused_step = outcome.steps_of(source).next().unwrap().clone();
        assert!(!refused_step.admission.is_admitted());
        let seq = refused_step_seq(&outcome, &refused_step);
        let mut ro = outcome.observer;
        let packets: Vec<_> = ro
            .runtime_mut()
            .drain_inline()
            .into_iter()
            .filter(|e| u64::from(e.payload.generation) == seq)
            .collect();
        let phases: Vec<u64> = packets.iter().map(|e| e.key.phase()).collect();
        assert!(phases.contains(&(BowlPhase::Prepared as u64)));
        assert!(phases.contains(&(BowlPhase::Refused as u64)));
        assert!(!phases.contains(&(BowlPhase::Admitted as u64)));
        assert!(!phases.contains(&(BowlPhase::Committed as u64)));
        for p in &packets {
            let mask = p.key.field_mask() as u16;
            if p.key.phase() != BowlPhase::Prepared as u64 {
                assert_ne!(mask & REFUSED_BIT, 0);
            }
            assert_eq!(mask & COMMITTED_BIT, 0);
        }
        // The program's own observer saw nothing of the refused attempt: prepare does not call it.
        let f = outcome.programs.into_iter().next().unwrap();
        let committed = f.steps as usize;
        assert_eq!(f.observer.published(), committed * 6, "{}", policy.name());
        if policy == AdmissionPolicy::Consume {
            assert_eq!((f.steps, f.refused), (0, 1));
            assert_eq!(f.observer.published(), 0);
        }
    }
}

fn refused_step_seq<O, W>(
    outcome: &big_bad_bool_runtime::RunOutcome<O, W>,
    step: &big_bad_bool_runtime::StepRecord,
) -> u64 {
    outcome
        .log
        .iter()
        .find(|e| e.step() == Some(step))
        .unwrap()
        .seq
}
