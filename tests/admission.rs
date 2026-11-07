//! Atomic admission and every bound's refusal.

mod common;

use big_bad_bool_runtime::fixtures::{
    HIT, START, TAG_MIXED, alarm, broadcaster, broadcaster_table, flood_table, mixed_table, rally,
    rally_table, signal_message, sink, strict_sink, supervisor, supervisor_table, twice_table,
    two_timers_table, verified, volley,
};
use big_bad_bool_runtime::{
    ActionRecord, Admission, AdmissionPolicy, BuildError, Delivery, Disposition, EffectTable, Port,
    ProgramId, ProgramInstance, ProgramSlot, Route, Runtime, RuntimeConfig, RuntimeEffect,
    RuntimeEventKind, RuntimeLimits, RuntimeRefusal,
};

use common::{fifo, id, refused, start_digest, steps};

#[test]
fn atomic_admission_several_actions_some_fit_alone_none_is_committed_nothing_is_performed() {
    // `Arm` asks for a Hit to the sink (one free place: fits), two timers (one free place: one
    // would fit alone, two do not) and an external word (always fits).
    let (a, s) = (verified(alarm()), verified(sink()));
    let phase = a.spec().dimension_id("phase").unwrap();
    let (alarm_id, sink_id) = (id("alarm"), id("sink"));
    let run = |timers: usize| {
        Runtime::builder(fifo(|l| l.timers = timers))
            .program(alarm_id, ProgramInstance::new(&a, mixed_table()))
            .program(sink_id, ProgramInstance::new(&s, EffectTable::new()))
            .route(Route::new(alarm_id, Port(0), sink_id))
            .inject(alarm_id, signal_message(START))
            .build()
            .unwrap()
            .run()
    };
    let outcome = run(1);
    let step = outcome.steps_of(alarm_id).next().unwrap();
    assert_eq!(step.effects.len(), 4);
    assert_eq!(
        refused(step),
        [RuntimeRefusal::TooManyTimers { needed: 2, free: 1 }]
    );
    // Nothing committed: the digest is the start digest before and after, the phase is Idle.
    assert_eq!(step.pre, start_digest(&a));
    assert_eq!(step.post, step.pre);
    assert_eq!(outcome.program(alarm_id).unwrap().state.word(phase), 0);
    // Nothing performed: no action, no delivery to the sink, no timer, no external word, no
    // message numbered beyond the injection.
    assert!(step.actions.is_empty());
    assert_eq!(outcome.program(sink_id).unwrap().steps, 0);
    assert!(outcome.steps_of(sink_id).next().is_none());
    assert!(
        !outcome
            .log
            .iter()
            .any(|e| matches!(e.kind, RuntimeEventKind::TimerFired { .. }))
    );
    assert!(outcome.external.is_empty());
    assert_eq!(outcome.messages, 1);
    // One event for the whole decision.
    assert_eq!(steps(&outcome).len(), 1);
    // With two timer places the same step commits and performs all four, in order.
    let outcome = run(2);
    let step = outcome.steps_of(alarm_id).next().unwrap();
    assert_eq!(step.admission, Admission::Admitted);
    assert!(matches!(
        step.actions.as_slice(),
        [
            ActionRecord::Sent { .. },
            ActionRecord::TimerArmed { .. },
            ActionRecord::TimerArmed { .. },
            ActionRecord::External { tag: TAG_MIXED, .. }
        ]
    ));
    assert_eq!(
        outcome
            .external
            .iter()
            .filter(|e| e.tag == TAG_MIXED)
            .count(),
        1
    );
}

#[test]
fn every_refusal_reason_is_collected_in_one_event() {
    let p = verified(rally());
    let (a, b) = (id("a"), id("b"));
    // 17 effects (one past 16), and the Send's port has no route: both in one record.
    let outcome = Runtime::builder(fifo(|_| {}))
        .program(a, ProgramInstance::new(&p, flood_table()))
        .program(b, ProgramInstance::new(&p, rally_table()))
        .inject(a, signal_message(HIT))
        .build()
        .unwrap()
        .run();
    let s = steps(&outcome);
    assert_eq!(
        refused(s[0]),
        [
            RuntimeRefusal::EffectsPerStep {
                observed: 17,
                permitted: 16
            },
            RuntimeRefusal::NoRoute { port: Port(0) },
        ]
    );
    assert_eq!(s[0].pre, s[0].post);
}

#[test]
fn bound_programs_routes_fanout_and_backlog_refuse_at_build() {
    let p = verified(sink());
    let limits = |f: fn(&mut RuntimeLimits)| {
        let mut l = RuntimeLimits::default();
        f(&mut l);
        RuntimeConfig::default().limits(l)
    };
    let three = |config: RuntimeConfig| {
        let mut b = Runtime::builder(config);
        for i in 0..3 {
            b = b.program(
                id(&format!("p{i}")),
                ProgramInstance::new(&p, EffectTable::new()),
            );
        }
        b.route(Route::new(id("p0"), Port(0), id("p1")))
            .route(Route::new(id("p0"), Port(0), id("p2")))
            .route(Route::new(id("p1"), Port(0), id("p2")))
    };
    assert_eq!(
        three(limits(|l| l.programs = 2)).build().err(),
        Some(BuildError::TooManyPrograms {
            observed: 3,
            permitted: 2
        })
    );
    assert_eq!(
        three(limits(|l| l.routes = 2)).build().err(),
        Some(BuildError::TooManyRoutes {
            observed: 3,
            permitted: 2
        })
    );
    assert_eq!(
        three(limits(|l| l.fanout = 1)).build().err(),
        Some(BuildError::FanOut {
            from: id("p0"),
            port: Port(0),
            observed: 2,
            permitted: 1
        })
    );
    assert_eq!(
        three(limits(|l| l.retry_backlog = 3).policy(AdmissionPolicy::retry(4)))
            .build()
            .err(),
        Some(BuildError::Backlog {
            program: None,
            requested: 4,
            permitted: 3
        })
    );
    let own = Runtime::builder(limits(|l| l.retry_backlog = 3))
        .program(
            id("p0"),
            ProgramInstance::new(&p, EffectTable::new()).policy(AdmissionPolicy::retry(9)),
        )
        .build()
        .err();
    assert_eq!(
        own,
        Some(BuildError::Backlog {
            program: Some(id("p0")),
            requested: 9,
            permitted: 3
        })
    );
    assert!(
        three(limits(|l| {
            l.programs = 3;
            l.routes = 3;
            l.fanout = 2;
        }))
        .build()
        .is_ok()
    );
    let unknown = Runtime::builder(RuntimeConfig::default())
        .program(id("p0"), ProgramInstance::new(&p, EffectTable::new()))
        .inject(id("nobody"), signal_message(HIT))
        .build()
        .err();
    assert_eq!(unknown, Some(BuildError::UnknownProgram(id("nobody"))));
}

#[test]
fn bound_mailbox_refuses_injections_and_steps() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let (source, target) = (id("source"), id("target"));
    let outcome = Runtime::builder(fifo(|l| l.mailbox = 1))
        .program(source, ProgramInstance::new(&caster, broadcaster_table()))
        .program(target, ProgramInstance::new(&s, EffectTable::new()))
        .route(Route::new(source, Port(0), target))
        .inject(target, signal_message(HIT))
        .inject(target, signal_message(HIT))
        .inject(source, signal_message(START))
        .build()
        .unwrap()
        .run();
    let injected: Vec<&Delivery> = outcome
        .log
        .iter()
        .filter_map(|e| match &e.kind {
            RuntimeEventKind::Injected { delivery } => Some(delivery),
            _ => None,
        })
        .collect();
    assert!(matches!(injected[0], Delivery::Delivered { .. }));
    assert!(matches!(
        injected[1],
        Delivery::Refused {
            reason: RuntimeRefusal::MailboxFull {
                needed: 1,
                free: 0,
                ..
            },
            ..
        }
    ));
    let first = outcome.steps_of(source).next().unwrap();
    assert_eq!(
        refused(first),
        [RuntimeRefusal::MailboxFull {
            to: ProgramSlot(1),
            needed: 1,
            free: 0
        }]
    );
}

#[test]
fn bound_deliveries_per_step() {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let count = s.spec().dimension_id("count").unwrap();
    let hub = id("hub");
    let sinks: Vec<ProgramId> = (0..8).map(|n| id(&format!("sink {n}"))).collect();
    let run = |deliveries| {
        let mut b = Runtime::builder(fifo(|l| l.deliveries_per_step = deliveries))
            .program(hub, ProgramInstance::new(&caster, twice_table()));
        for t in &sinks {
            b = b
                .program(*t, ProgramInstance::new(&s, EffectTable::new()))
                .route(Route::new(hub, Port(0), *t));
        }
        b.inject(hub, signal_message(START)).build().unwrap().run()
    };
    let refused_run = run(10);
    assert_eq!(
        refused(steps(&refused_run)[0]),
        [RuntimeRefusal::FanOut {
            observed: 16,
            permitted: 10
        }]
    );
    for t in &sinks {
        assert_eq!(refused_run.program(*t).unwrap().state.word(count), 0);
    }
    let admitted = run(16);
    for t in &sinks {
        assert_eq!(admitted.program(*t).unwrap().state.word(count), 2);
    }
}

#[test]
fn bound_effects_per_step() {
    let p = verified(rally());
    let (a, b) = (id("a"), id("b"));
    let run = |effects| {
        Runtime::builder(fifo(|l| l.effects_per_step = effects))
            .program(a, ProgramInstance::new(&p, flood_table()))
            .program(b, ProgramInstance::new(&p, rally_table()))
            .route(Route::new(a, Port(0), b))
            .inject(a, signal_message(HIT))
            .build()
            .unwrap()
            .run()
    };
    let outcome = run(16);
    assert_eq!(
        refused(steps(&outcome)[0]),
        [RuntimeRefusal::EffectsPerStep {
            observed: 17,
            permitted: 16
        }]
    );
    assert_eq!(outcome.program(a).unwrap().digest, start_digest(&p));
    assert_eq!(outcome.program(b).unwrap().steps, 0);
    assert_eq!(steps(&run(17))[0].admission, Admission::Admitted);
}

#[test]
fn bound_timers() {
    let p = verified(alarm());
    let phase = p.spec().dimension_id("phase").unwrap();
    let alarm_id = id("alarm");
    let run = |timers| {
        Runtime::builder(fifo(|l| l.timers = timers))
            .program(alarm_id, ProgramInstance::new(&p, two_timers_table()))
            .inject(alarm_id, signal_message(START))
            .build()
            .unwrap()
            .run()
    };
    let one = run(1);
    assert_eq!(
        refused(steps(&one)[0]),
        [RuntimeRefusal::TooManyTimers { needed: 2, free: 1 }]
    );
    assert_eq!(one.program(alarm_id).unwrap().state.word(phase), 0);
    let two = run(2);
    assert_eq!(
        two.log
            .iter()
            .filter(|e| matches!(e.kind, RuntimeEventKind::TimerFired { .. }))
            .count(),
        2
    );
    assert_eq!(two.program(alarm_id).unwrap().state.word(phase), 2);
}

#[test]
fn bound_children() {
    let p = verified(supervisor());
    let sup = id("supervisor");
    let outcome = Runtime::builder(fifo(|l| l.children = 0))
        .program(sup, ProgramInstance::new(&p, supervisor_table()))
        .inject(sup, signal_message(START))
        .build()
        .unwrap()
        .run();
    assert_eq!(
        refused(steps(&outcome)[0]),
        [RuntimeRefusal::TooManyChildren { needed: 1, free: 0 }]
    );
    assert_eq!(outcome.program(sup).unwrap().digest, start_digest(&p));
}

#[test]
fn bound_steps_ends_an_endless_volley() {
    let p = verified(volley());
    let (a, b) = (id("a"), id("b"));
    let outcome = Runtime::builder(fifo(|l| l.steps = 50))
        .program(a, ProgramInstance::new(&p, rally_table()))
        .program(b, ProgramInstance::new(&p, rally_table()))
        .route(Route::new(a, Port(0), b))
        .route(Route::new(b, Port(0), a))
        .inject(a, signal_message(HIT))
        .build()
        .unwrap()
        .run();
    assert_eq!(outcome.steps, 50);
    let last = steps(&outcome).into_iter().last().unwrap();
    assert_eq!(
        refused(last),
        [RuntimeRefusal::StepBudget { permitted: 50 }]
    );
    assert_eq!(
        last.disposition,
        Disposition::Consumed {
            final_refusal: true
        }
    );
}

#[test]
fn bound_trace_events_drops_the_oldest_and_keeps_numbers() {
    let p = verified(rally());
    let full = common::rally_pairs(&p, 2, RuntimeConfig::default());
    let kept = common::rally_pairs(
        &p,
        2,
        RuntimeConfig::default().limits(RuntimeLimits {
            trace_events: 10,
            ..RuntimeLimits::default()
        }),
    );
    assert_eq!(kept.log.len(), 10);
    assert_eq!(kept.events, full.events);
    assert_eq!(kept.trace_dropped, full.events - 10);
    // The kept events are the last ten, with their own sequence numbers.
    assert_eq!(kept.log[..], full.log[full.log.len() - 10..]);
    assert_eq!(kept.log[0].seq, kept.trace_dropped);
    assert_eq!(kept.scheduler.len(), 10);
    assert!(kept.scheduler_dropped > 0);
    assert_eq!(kept.final_digests(), full.final_digests());
}

#[test]
fn a_finished_receiver_refuses_the_step_that_would_send_to_it() {
    let (caster, strict) = (verified(broadcaster()), verified(strict_sink()));
    let (hub, target) = (id("hub"), id("strict"));
    let outcome = Runtime::builder(fifo(|_| {}))
        .program(target, ProgramInstance::new(&strict, EffectTable::new()))
        .program(hub, ProgramInstance::new(&caster, broadcaster_table()))
        .route(Route::new(hub, Port(0), target))
        .inject(target, signal_message(START))
        .inject_in_epoch(1, hub, signal_message(START))
        .build()
        .unwrap()
        .run();
    assert!(outcome.program(target).unwrap().halted);
    let hub_step = outcome.steps_of(hub).next().unwrap();
    assert_eq!(
        refused(hub_step),
        [RuntimeRefusal::TargetFinished { to: ProgramSlot(0) }]
    );
    assert_eq!(outcome.program(hub).unwrap().digest, start_digest(&caster));
}

#[test]
fn an_effect_table_must_fit_its_program() {
    let p = verified(sink());
    let err = Runtime::builder(RuntimeConfig::default())
        .program(
            id("s"),
            ProgramInstance::new(
                &p,
                EffectTable::new().on("count", 1, [RuntimeEffect::Trace { tag: 0 }]),
            ),
        )
        .build()
        .err();
    assert_eq!(
        err,
        Some(BuildError::EffectTable {
            program: id("s"),
            error: big_bad_bool_runtime::TableError::NotAnEffect("count")
        })
    );
}
