//! The scenarios: ping-pong, fan-out, logical timer, cancellation, contention, isolation, UUID
//! topology.

mod common;

use big_bad_bool_runtime::fixtures::{
    ALARM_TICKS, HIT, START, TAG_RALLY_DONE, TAG_RING, TICKER_TICKS, alarm, alarm_table,
    broadcaster, broadcaster_table, rally, rally_table, signal_message, sink, supervisor,
    supervisor_table, verified,
};
use big_bad_bool_runtime::{
    ActionRecord, Admission, BuildError, ChildKey, Delivery, EffectTable, Origin,
    PROGRAM_NAMESPACE, Port, ProgramId, ProgramInstance, ProgramSlot, ROUTE_NAMESPACE, Route,
    RouteId, RunId, Runtime, RuntimeConfig, RuntimeEffect, RuntimeEventKind, RuntimeLimits,
    RuntimeMessage, SchedEvent, TimerId,
};
use scope::{SchedulerConfig, TaskState, Tick, WakeOrdering, WakeReason};
use uuid_macros::uuid::Uuid;

use common::{
    BUDGETS, POLICIES, casters, id, rally_pairs, scheduler, standalone, start_digest, steps,
};

#[test]
fn ping_pong_alternates_and_keeps_states_apart() {
    let program = verified(rally());
    let count = program.spec().dimension_id("count").unwrap();
    let start = start_digest(&program);
    let outcome = rally_pairs(&program, 1, RuntimeConfig::default());
    let (a, b) = common::pair_ids(0);
    let (sa, sb) = (ProgramSlot(0), ProgramSlot(1));

    let s = steps(&outcome);
    let order: Vec<ProgramSlot> = s.iter().map(|r| r.program).collect();
    assert_eq!(order, [sa, sb, sa, sb, sa, sb, sa, sb, sa]);
    assert!(s.iter().all(|r| r.admission == Admission::Admitted));
    assert_ne!(s[0].pre, s[0].post);
    assert_eq!(
        s[1].pre, start,
        "B is still at its start state after A's first commit"
    );
    for pair in s.windows(2) {
        let sent: Vec<_> = pair[0]
            .actions
            .iter()
            .flat_map(|a| match a {
                ActionRecord::Sent { deliveries, .. } => deliveries.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            sent,
            [Delivery::Delivered {
                to: pair[1].program,
                message: pair[1].message
            }]
        );
        assert_eq!(pair[1].origin, Origin::Program(pair[0].program));
    }
    for w in outcome.steps_of(a).collect::<Vec<_>>().windows(2) {
        assert_eq!(w[0].post, w[1].pre);
    }
    let (fa, fb) = (outcome.program(a).unwrap(), outcome.program(b).unwrap());
    assert_eq!((fa.state.word(count), fb.state.word(count)), (4, 4));
    assert_eq!((fa.steps, fb.steps), (5, 4));
    assert_eq!(outcome.external.len(), 1);
    assert_eq!(outcome.external[0].tag, TAG_RALLY_DONE);
    let hits = |n| vec![signal_message(HIT); n];
    assert_eq!(fa.digest, standalone(&program, &hits(5)));
    assert_eq!(fb.digest, standalone(&program, &hits(4)));
    // Every event is numbered in order, with no gap.
    assert!(
        outcome
            .log
            .iter()
            .enumerate()
            .all(|(i, e)| e.seq == i as u64)
    );
    assert_eq!(outcome.events, outcome.log.len() as u64);
}

#[test]
fn fan_out_follows_route_order_within_its_bound() {
    let caster = verified(broadcaster());
    let sink_program = verified(sink());
    let count = sink_program.spec().dimension_id("count").unwrap();
    let hub = id("hub");
    let sinks: Vec<ProgramId> = [5, 2, 8, 1, 7, 3, 6, 4]
        .map(|n| id(&format!("sink {n}")))
        .to_vec();
    let build = |config: RuntimeConfig, sinks: &[ProgramId]| {
        let mut b = Runtime::builder(config)
            .program(hub, ProgramInstance::new(&caster, broadcaster_table()));
        for s in sinks {
            b = b
                .program(*s, ProgramInstance::new(&sink_program, EffectTable::new()))
                .route(Route::new(hub, Port(0), *s));
        }
        b.inject(hub, signal_message(START)).build()
    };
    for ordering in WakeOrdering::ALL {
        let config = RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(ordering));
        let outcome = build(config, &sinks).unwrap().run();
        let first = steps(&outcome)[0];
        let [ActionRecord::Sent { deliveries, .. }] = first.actions.as_slice() else {
            panic!()
        };
        let to: Vec<ProgramId> = deliveries
            .iter()
            .map(|d| match d {
                Delivery::Delivered { to, .. } => outcome.program_ids[to.0 as usize],
                Delivery::Refused { .. } => panic!("{d:?}"),
            })
            .collect();
        assert_eq!(to, sinks, "{}", ordering.name());
        for s in &sinks {
            assert_eq!(outcome.program(*s).unwrap().state.word(count), 1);
        }
    }
    let limits = |fanout| {
        RuntimeConfig::default().limits(RuntimeLimits {
            fanout,
            ..RuntimeLimits::default()
        })
    };
    assert_eq!(
        build(limits(7), &sinks).err(),
        Some(BuildError::FanOut {
            from: hub,
            port: Port(0),
            observed: 8,
            permitted: 7
        })
    );
    assert!(build(limits(8), &sinks).is_ok());
}

#[test]
fn a_logical_timer_fires_at_its_tick_and_repeats_exactly() {
    let program = verified(alarm());
    let phase = program.spec().dimension_id("phase").unwrap();
    let alarm_id = id("alarm");
    let run = || {
        Runtime::builder(RuntimeConfig::default())
            .program(alarm_id, ProgramInstance::new(&program, alarm_table()))
            .inject(alarm_id, signal_message(START))
            .build()
            .unwrap()
            .run()
    };
    let outcome = run();
    let s = steps(&outcome);
    assert_eq!(s.len(), 2);
    let ActionRecord::TimerArmed {
        timer,
        message,
        after,
    } = s[0].actions[0]
    else {
        panic!("{:?}", s[0].actions)
    };
    assert_eq!(after, ALARM_TICKS.0);
    // The timer task is spawned after the arming poll and arms Scope's tick timer on its own
    // first poll, one round later; it fires ALARM_TICKS rounds after that.
    let armed = outcome
        .log
        .iter()
        .find(|e| e.step().is_some())
        .unwrap()
        .tick;
    let fired = Tick(armed.0 + 1 + ALARM_TICKS.0);
    let fired_event = outcome
        .log
        .iter()
        .find(|e| matches!(e.kind, RuntimeEventKind::TimerFired { .. }))
        .unwrap();
    assert_eq!(fired_event.tick, fired);
    assert_eq!(
        fired_event.kind,
        RuntimeEventKind::TimerFired {
            program: ProgramSlot(0),
            timer,
            delivery: Delivery::Delivered {
                to: ProgramSlot(0),
                message
            }
        }
    );
    assert_eq!(s[1].message, message);
    assert_eq!(s[1].origin, Origin::Timer(ProgramSlot(0), timer));
    assert_eq!(timer, TimerId::of(alarm_id, 0));
    assert_eq!(
        s[1].actions,
        [ActionRecord::External {
            tag: TAG_RING,
            word: 4
        }]
    );
    assert_eq!(
        outcome.program(alarm_id).unwrap().state.word(phase),
        2,
        "Done"
    );
    assert!(
        outcome
            .scheduler
            .iter()
            .any(|e| matches!(e, SchedEvent::TickTimer(_, t) if *t == fired))
    );
    assert!(
        outcome
            .scheduler
            .iter()
            .any(|e| matches!(e, SchedEvent::Wake(_, WakeReason::Timer)))
    );
    // Identical ticks and trace over repeats.
    for _ in 0..10 {
        let again = run();
        assert!(again.same_run(&outcome), "{:?}", again.diff(&outcome));
    }
}

#[test]
fn a_committed_effect_cancels_a_scoped_child() {
    let program = verified(supervisor());
    let count = program.spec().dimension_id("count").unwrap();
    let sup = id("supervisor");
    let outcome = Runtime::builder(RuntimeConfig::default())
        .program(sup, ProgramInstance::new(&program, supervisor_table()))
        .inject(sup, signal_message(START))
        .build()
        .unwrap()
        .run();

    let ticks = outcome
        .log
        .iter()
        .filter(|e| matches!(e.kind, RuntimeEventKind::ChildTick { .. }))
        .count();
    assert!((3..TICKER_TICKS as usize).contains(&ticks), "{ticks}");
    let cancel = steps(&outcome)
        .into_iter()
        .find_map(|s| match s.actions.as_slice() {
            [
                ActionRecord::ChildCancelRequested {
                    key,
                    task: Some(task),
                },
            ] => Some((*key, *task, s.post.clone(), s.pre.clone())),
            _ => None,
        })
        .expect("tick-3 cancels the child");
    assert_eq!(cancel.0, ChildKey(0));
    assert_ne!(cancel.2, cancel.3, "the cancel came from a commit");
    let child = cancel.1;
    let req = outcome
        .scheduler
        .iter()
        .position(|e| matches!(e, SchedEvent::CancelRequest(t, _, Some(2)) if *t == child));
    let fin = outcome
        .scheduler
        .iter()
        .position(|e| *e == SchedEvent::Finish(child, TaskState::Cancelled));
    assert!(req.is_some() && fin.is_some() && req < fin);
    let ended = outcome
        .log
        .iter()
        .position(|e| {
            e.kind
                == RuntimeEventKind::ChildEnded {
                    program: ProgramSlot(0),
                    key: ChildKey(0),
                    cancelled: true,
                }
        })
        .expect("the child ended cancelled");
    assert_eq!(outcome.runs[0].cancelled, 1);
    assert_eq!(outcome.program(sup).unwrap().state.word(count), 4);
    assert!(
        !outcome.log[ended..]
            .iter()
            .any(|e| matches!(e.kind, RuntimeEventKind::ChildTick { .. }))
    );
}

#[test]
fn contention_is_deterministic_under_every_scope_policy_budget_and_admission_policy() {
    let program = verified(rally());
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let mut finals = Vec::new();
    let mut logs = Vec::new();
    for ordering in WakeOrdering::ALL {
        for budget in BUDGETS {
            let sched = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
            let first = rally_pairs(&program, 16, scheduler(sched));
            assert_eq!((first.steps, first.refused), (16 * 9, 0));
            for _ in 0..4 {
                let again = rally_pairs(&program, 16, scheduler(sched));
                assert!(
                    again.same_run(&first),
                    "{} budget {budget}: {:?}",
                    ordering.name(),
                    again.diff(&first)
                );
            }
            finals.push(first.final_digests());
            logs.push(first.log);
            for policy in POLICIES {
                let config = scheduler(sched).policy(policy).limits(RuntimeLimits {
                    mailbox: 2,
                    ..RuntimeLimits::default()
                });
                let first = casters(&caster, &s, config);
                assert!(
                    first.refused > 0,
                    "{} {budget} {}",
                    ordering.name(),
                    policy.name()
                );
                for _ in 0..4 {
                    let again = casters(&caster, &s, config);
                    assert!(
                        again.same_run(&first),
                        "{} budget {budget} {}: {:?}",
                        ordering.name(),
                        policy.name(),
                        again.diff(&first)
                    );
                }
            }
        }
    }
    // Program states do not depend on the scheduler; the interleaving does.
    assert!(finals.windows(2).all(|w| w[0] == w[1]));
    let distinct = logs
        .iter()
        .enumerate()
        .filter(|(i, l)| logs[..*i].iter().all(|m| m != *l))
        .count();
    assert!(distinct > 1, "{distinct}");
}

fn assert_plain_data<T: 'static + Send + Sync + Clone + Eq>() {}

#[test]
fn isolation_programs_reach_only_messages() {
    assert_plain_data::<RuntimeMessage>();
    assert_plain_data::<RuntimeEffect>();
    assert_plain_data::<EffectTable>();
    assert_plain_data::<ProgramId>();
    let program = verified(sink());
    let (a, b) = (id("a"), id("b"));
    let outcome = Runtime::builder(RuntimeConfig::default())
        .program(a, ProgramInstance::new(&program, EffectTable::new()))
        .program(b, ProgramInstance::new(&program, EffectTable::new()))
        .inject(a, signal_message(HIT))
        .inject(a, signal_message(HIT))
        .inject(a, signal_message(HIT))
        .inject(b, signal_message(HIT))
        .build()
        .unwrap()
        .run();
    let (fa, fb) = (outcome.program(a).unwrap(), outcome.program(b).unwrap());
    assert_ne!(fa.digest, fb.digest);
    assert_eq!(
        fa.digest,
        standalone(&program, &vec![signal_message(HIT); 3])
    );
    assert_eq!(fb.digest, standalone(&program, &[signal_message(HIT)]));
}

#[test]
fn uuid_topology_same_names_same_ids_and_slots_stay_internal() {
    let p = verified(rally());
    let build = |names: [&str; 2]| {
        let (a, b) = (id(names[0]), id(names[1]));
        Runtime::builder(RuntimeConfig::default())
            .program(a, ProgramInstance::new(&p, rally_table()))
            .program(b, ProgramInstance::new(&p, rally_table()))
            .route(Route::new(a, Port(0), b))
            .route(Route::new(b, Port(0), a))
            .inject(a, signal_message(HIT))
            .build()
            .unwrap()
    };
    let (one, two) = (build(["ping", "pong"]), build(["ping", "pong"]));
    assert_eq!(one.program_ids(), two.program_ids());
    assert_eq!(one.route_ids(), two.route_ids());
    assert_eq!(one.run_id(), two.run_id());
    // Fixed values: v5 under the crate's own namespaces, the same on every machine.
    let ping = id("ping");
    assert_eq!(ping.get_version_num(), 5);
    let ns = |kind: &str| Uuid::new_v5(&Uuid::NAMESPACE_OID, kind.as_bytes());
    assert_eq!(ping.0, Uuid::new_v5(&ns(PROGRAM_NAMESPACE), b"ping"));
    let route = RouteId::of(ping, Port(0), id("pong"));
    assert_eq!(
        route.0,
        Uuid::new_v5(
            &ns(ROUTE_NAMESPACE),
            format!("{ping}/0/{}", id("pong")).as_bytes()
        )
    );
    assert_eq!(route, one.route_ids()[0]);
    // Kinds never share a namespace: the same name gives four different ids, and none is the
    // OID-namespace `named` id.
    let kinds = [
        ProgramId::from_name("x").0,
        RouteId::named_v5(ROUTE_NAMESPACE, "x").0,
        TimerId::named_v5(big_bad_bool_runtime::TIMER_NAMESPACE, "x").0,
        RunId::of_topology("x").0,
        ProgramId::named("x").0,
    ];
    for (i, a) in kinds.iter().enumerate() {
        for b in &kinds[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_eq!(TimerId::of(ping, 0), TimerId::of(ping, 0));
    assert_ne!(TimerId::of(ping, 0), TimerId::of(ping, 1));
    // Slots are registration order, nothing more: swap the order and the ids, final digests and
    // external words by id are the same; only the slots (and so the run id) change.
    let (r1, r2) = (build(["ping", "pong"]).run(), build(["ping", "pong"]).run());
    assert!(r1.same_run(&r2));
    let swapped = {
        let (a, b) = (id("ping"), id("pong"));
        Runtime::builder(RuntimeConfig::default())
            .program(b, ProgramInstance::new(&p, rally_table()))
            .program(a, ProgramInstance::new(&p, rally_table()))
            .route(Route::new(a, Port(0), b))
            .route(Route::new(b, Port(0), a))
            .inject(a, signal_message(HIT))
            .build()
            .unwrap()
    };
    assert_ne!(swapped.run_id(), one.run_id());
    let swapped = swapped.run();
    assert_eq!(r1.slot(ping), Some(ProgramSlot(0)));
    assert_eq!(swapped.slot(ping), Some(ProgramSlot(1)));
    for name in ["ping", "pong"] {
        assert_eq!(
            r1.program(id(name)).unwrap().digest,
            swapped.program(id(name)).unwrap().digest
        );
    }
    assert_eq!(r1.external.len(), swapped.external.len());
    // Another program under the same names: another run id.
    let s = verified(sink());
    let other = Runtime::builder(RuntimeConfig::default())
        .program(ping, ProgramInstance::new(&s, EffectTable::new()))
        .build()
        .unwrap();
    let same_name_rally = Runtime::builder(RuntimeConfig::default())
        .program(ping, ProgramInstance::new(&p, EffectTable::new()))
        .build()
        .unwrap();
    assert_ne!(other.run_id(), same_name_rally.run_id());
    let _: RunId = r1.run_id;
}
