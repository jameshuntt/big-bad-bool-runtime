//! Replay: identical inputs and configuration give the identical run, timers included; a
//! changed run reports its first difference.

mod common;

use big_bad_bool_runtime::fixtures::{
    HIT, START, alarm, alarm_table, metronome, metronome_table, rally, rally_table, signal_message,
    supervisor, supervisor_table, verified,
};
use big_bad_bool_runtime::{
    Port, ProgramInstance, Route, RunDiff, Runtime, RuntimeConfig, RuntimeEventKind, RuntimeLimits,
    SchedEvent, TraceLevel,
};
use scope::{SchedulerConfig, Tick, WakeOrdering};

use common::{BUDGETS, POLICIES, id, rally_pairs, scheduler};

fn firing_ticks<O, W>(outcome: &big_bad_bool_runtime::RunOutcome<O, W>) -> Vec<Tick> {
    outcome
        .log
        .iter()
        .filter(|e| matches!(e.kind, RuntimeEventKind::TimerFired { .. }))
        .map(|e| e.tick)
        .collect()
}

fn wakes<O, W>(outcome: &big_bad_bool_runtime::RunOutcome<O, W>) -> Vec<SchedEvent> {
    outcome
        .scheduler
        .iter()
        .filter(|e| matches!(e, SchedEvent::Wake(..) | SchedEvent::TickTimer(..)))
        .cloned()
        .collect()
}

#[test]
fn logical_timer_runs_repeat_exactly_under_every_policy_and_budget() {
    let (a, m, r) = (verified(alarm()), verified(metronome()), verified(rally()));
    for ordering in WakeOrdering::ALL {
        for budget in BUDGETS {
            for policy in POLICIES {
                let config =
                    scheduler(SchedulerConfig::with_ordering(ordering).with_poll_budget(budget))
                        .policy(policy)
                        .limits(RuntimeLimits {
                            steps: 60,
                            ..RuntimeLimits::default()
                        });
                let run = || {
                    let (x, y) = (id("x"), id("y"));
                    Runtime::builder(config)
                        .program(id("alarm 1"), ProgramInstance::new(&a, alarm_table()))
                        .program(id("alarm 2"), ProgramInstance::new(&a, alarm_table()))
                        .program(id("metronome"), ProgramInstance::new(&m, metronome_table()))
                        .program(x, ProgramInstance::new(&r, rally_table()))
                        .program(y, ProgramInstance::new(&r, rally_table()))
                        .route(Route::new(x, Port(0), y))
                        .route(Route::new(y, Port(0), x))
                        .inject(id("alarm 1"), signal_message(START))
                        .inject(id("metronome"), signal_message(START))
                        .inject(x, signal_message(HIT))
                        .inject_in_epoch(1, id("alarm 2"), signal_message(START))
                        .build()
                        .unwrap()
                        .run()
                };
                let first = run();
                assert!(firing_ticks(&first).len() >= 3);
                for _ in 0..4 {
                    let again = run();
                    assert!(
                        again.same_run(&first),
                        "{} {budget} {}: {:?}",
                        ordering.name(),
                        policy.name(),
                        again.diff(&first)
                    );
                    assert_eq!(wakes(&again), wakes(&first));
                    assert_eq!(firing_ticks(&again), firing_ticks(&first));
                    assert_eq!(
                        again.runs.iter().map(|r| r.rounds).collect::<Vec<_>>(),
                        first.runs.iter().map(|r| r.rounds).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn ping_pong_twenty_runs_are_one_run() {
    let p = verified(rally());
    let first = rally_pairs(&p, 1, RuntimeConfig::default());
    for _ in 0..19 {
        assert!(rally_pairs(&p, 1, RuntimeConfig::default()).same_run(&first));
    }
}

#[test]
fn cancellation_is_deterministic_under_every_policy() {
    let p = verified(supervisor());
    for ordering in WakeOrdering::ALL {
        for budget in BUDGETS {
            let config =
                scheduler(SchedulerConfig::with_ordering(ordering).with_poll_budget(budget));
            let run = || {
                let (one, two) = (id("one"), id("two"));
                Runtime::builder(config)
                    .program(one, ProgramInstance::new(&p, supervisor_table()))
                    .program(two, ProgramInstance::new(&p, supervisor_table()))
                    .inject(one, signal_message(START))
                    .inject(two, signal_message(START))
                    .build()
                    .unwrap()
                    .run()
            };
            let first = run();
            for _ in 0..4 {
                assert!(run().same_run(&first), "{} {budget}", ordering.name());
            }
        }
    }
}

#[test]
fn trace_off_changes_no_result() {
    let p = verified(rally());
    let full = rally_pairs(&p, 8, RuntimeConfig::default());
    let off = rally_pairs(&p, 8, RuntimeConfig::default().trace(TraceLevel::Off));
    assert!(off.log.is_empty() && off.scheduler.is_empty());
    assert_eq!(off.events, 0);
    assert_eq!(off.final_digests(), full.final_digests());
    assert_eq!((off.steps, off.messages), (full.steps, full.messages));
    assert_eq!(off.external, full.external);
}

#[test]
fn a_different_run_reports_its_first_difference() {
    let p = verified(rally());
    let one = rally_pairs(&p, 2, RuntimeConfig::default());
    let fifo = rally_pairs(
        &p,
        2,
        RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Lifo)),
    );
    match one.diff(&fifo) {
        Some(RunDiff::Event {
            index,
            left: Some(l),
            right: Some(r),
        }) => {
            assert_eq!(one.log[..index], fifo.log[..index]);
            assert_ne!(l, r);
        }
        other => panic!("{other:?}"),
    }
    let other = rally_pairs(&p, 3, RuntimeConfig::default());
    assert_eq!(one.diff(&other), Some(RunDiff::RunId));
    assert_eq!(one.diff(&one_again(&p)), None);
}

fn one_again(
    p: &big_bad_bool::program::VerifiedProgram,
) -> big_bad_bool_runtime::RunOutcome<big_bad_bool::program::NoObserver> {
    rally_pairs(p, 2, RuntimeConfig::default())
}
