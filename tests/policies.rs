//! The admission policies: Consume, Retry, Backpressure.

mod common;

use big_bad_bool_runtime::fixtures::{
    HIT, START, alarm, broadcaster, broadcaster_table, rally_table, signal_message, sink,
    stalled_table, two_timers_table, verified, volley,
};
use big_bad_bool_runtime::{
    ActionRecord, Admission, AdmissionPolicy, Delivery, Disposition, EffectTable, InputSource,
    Origin, Port, ProgramInstance, ProgramSlot, Resource, Route, Runtime, RuntimeEventKind,
    RuntimeRefusal, StepRecord,
};
use scope::{SchedulerConfig, WakeOrdering};

use common::{BUDGETS, fifo, id, refused, scheduler, start_digest, steps};

/// A source (broadcaster) and a target (sink) whose `mailbox` places are full: two (or three)
/// hits are injected into the target first, then `starts` starts into the source. Under FIFO the
/// source is polled first, so its first send finds the target full.
fn source_and_full_target(
    policy: AdmissionPolicy,
    mailbox: usize,
    starts: usize,
    resend_in_epoch_1: bool,
) -> big_bad_bool_runtime::RunOutcome<big_bad_bool::program::NoObserver> {
    let caster = verified(broadcaster());
    let target_program = verified(sink());
    let (source, target) = (id("source"), id("target"));
    let mut b = Runtime::builder(fifo(|l| l.mailbox = mailbox).policy(policy))
        .program(source, ProgramInstance::new(&caster, broadcaster_table()))
        .program(target, ProgramInstance::new(&target_program, EffectTable::new()))
        .route(Route::new(source, Port(0), target));
    for _ in 0..mailbox {
        b = b.inject(target, signal_message(HIT));
    }
    for _ in 0..starts {
        b = b.inject(source, signal_message(START));
    }
    if resend_in_epoch_1 {
        b = b.inject_in_epoch(1, source, signal_message(START));
    }
    b.build().unwrap().run()
}

#[test]
fn consume_drops_the_refused_input_and_a_resend_in_a_later_epoch_commits() {
    let outcome = source_and_full_target(AdmissionPolicy::Consume, 2, 1, true);
    let caster = verified(broadcaster());
    let count = verified(sink()).spec().dimension_id("count").unwrap();
    let source_steps: Vec<&StepRecord> = outcome.steps_of(id("source")).collect();
    assert_eq!(source_steps.len(), 2);
    let (first, second) = (source_steps[0], source_steps[1]);
    // prepare succeeded: the step would commit `announce` and send one Hit.
    assert!(first.results[0].is_committed());
    assert_eq!(first.effects.len(), 1);
    // admission refused it: the target had no free place; the input is consumed.
    assert_eq!(
        refused(first),
        [RuntimeRefusal::MailboxFull { to: ProgramSlot(1), needed: 1, free: 0 }]
    );
    assert_eq!(first.disposition, Disposition::Consumed { final_refusal: false });
    assert_eq!(first.pre, start_digest(&caster));
    assert_eq!(first.post, first.pre);
    assert!(first.actions.is_empty());
    // epoch 1: the same transition, from a new message, commits.
    assert_ne!(second.message, first.message);
    assert_eq!(second.source, InputSource::Mailbox);
    assert_eq!((second.pre.clone(), second.results.clone(), second.effects.clone()), (first.pre.clone(), first.results.clone(), first.effects.clone()));
    assert_eq!(second.admission, Admission::Admitted);
    assert_eq!(outcome.program(id("target")).unwrap().state.word(count), 3);
    assert_eq!((outcome.refused, outcome.policy.consumed), (1, 1));
}

#[test]
fn retry_backlogs_the_refused_input_and_the_same_input_commits_once_a_place_is_freed() {
    let outcome = source_and_full_target(AdmissionPolicy::retry(4), 2, 1, false);
    let count = verified(sink()).spec().dimension_id("count").unwrap();
    let source_steps: Vec<&StepRecord> = outcome.steps_of(id("source")).collect();
    assert_eq!(source_steps.len(), 2, "{source_steps:#?}");
    let (first, second) = (source_steps[0], source_steps[1]);
    assert_eq!(refused(first).len(), 1);
    assert_eq!(first.disposition, Disposition::Backlogged { len: 1 });
    assert_eq!(first.pre, first.post);
    assert!(first.actions.is_empty());
    // The target's receive freed a place and woke the waiting source, in one epoch.
    let released = outcome
        .log
        .iter()
        .position(|e| matches!(&e.kind, RuntimeEventKind::Released { resource: Resource::Mailbox(ProgramSlot(1)), woken } if woken.len() == 1))
        .expect("the target released a place");
    let second_at = outcome.log.iter().position(|e| e.step() == Some(second)).unwrap();
    assert!(released < second_at);
    // The same input (same message id), retried: same pre-digest, results and effects; commits.
    assert_eq!(second.message, first.message);
    assert_eq!(second.source, InputSource::Backlog { attempt: 2 });
    assert_eq!(second.pre, first.pre);
    assert_eq!(second.results, first.results);
    assert_eq!(second.effects, first.effects);
    assert_eq!(second.admission, Admission::Admitted);
    assert_eq!(second.disposition, Disposition::Committed);
    assert!(!outcome.log.iter().any(|e| matches!(e.kind, RuntimeEventKind::Epoch { epoch: 1 })));
    assert_eq!(outcome.program(id("target")).unwrap().state.word(count), 3);
    let p = outcome.policy;
    assert_eq!((p.backlogged, p.retries, p.held_committed, p.consumed), (1, 1, 1, 0));
}

#[test]
fn retry_with_a_full_backlog_is_a_deterministic_refusal_event() {
    // No timer place is ever free (timers: 1, each Arm asks for 2): every Start stays refused.
    let p = verified(alarm());
    let alarm_id = id("alarm");
    let run = || {
        Runtime::builder(fifo(|l| l.timers = 1).policy(AdmissionPolicy::Retry { backlog: 1, max_attempts: u32::MAX }))
            .program(alarm_id, ProgramInstance::new(&p, two_timers_table()))
            .inject(alarm_id, signal_message(START))
            .inject(alarm_id, signal_message(START))
            .build()
            .unwrap()
            .run()
    };
    let outcome = run();
    let s = steps(&outcome);
    let shape: Vec<(InputSource, Disposition)> = s.iter().map(|r| (r.source, r.disposition)).collect();
    assert_eq!(
        shape,
        [
            (InputSource::Mailbox, Disposition::Backlogged { len: 1 }),
            (InputSource::Backlog { attempt: 2 }, Disposition::Kept { attempts: 2 }),
            (InputSource::Mailbox, Disposition::BacklogFull { capacity: 1 }),
        ]
    );
    assert!(s.iter().all(|r| r.pre == r.post && refused(r) == [RuntimeRefusal::TooManyTimers { needed: 2, free: 1 }]));
    // At the end, the held input is dropped with the program and named.
    let finished = outcome
        .log
        .iter()
        .find_map(|e| match &e.kind {
            RuntimeEventKind::Finished { held, .. } => Some(held.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(finished, [s[0].message]);
    assert_eq!(outcome.policy.backlog_full, 1);
    for _ in 0..5 {
        assert!(run().same_run(&outcome));
    }
}

#[test]
fn retry_drops_an_entry_after_max_attempts() {
    let p = verified(alarm());
    let alarm_id = id("alarm");
    let outcome = Runtime::builder(fifo(|l| l.timers = 1).policy(AdmissionPolicy::Retry { backlog: 2, max_attempts: 2 }))
        .program(alarm_id, ProgramInstance::new(&p, two_timers_table()))
        .inject(alarm_id, signal_message(START))
        .inject(alarm_id, signal_message(START))
        .build()
        .unwrap()
        .run();
    let shape: Vec<(InputSource, Disposition)> = steps(&outcome).iter().map(|r| (r.source, r.disposition)).collect();
    assert_eq!(
        shape,
        [
            (InputSource::Mailbox, Disposition::Backlogged { len: 1 }),
            (InputSource::Backlog { attempt: 2 }, Disposition::Exhausted { attempts: 2 }),
            (InputSource::Mailbox, Disposition::Backlogged { len: 1 }),
        ]
    );
    assert_eq!(outcome.policy.exhausted, 1);
}

#[test]
fn backpressure_holds_the_input_takes_nothing_else_and_commits_it_when_a_place_frees() {
    let outcome = source_and_full_target(AdmissionPolicy::Backpressure, 3, 3, false);
    let count = verified(sink()).spec().dimension_id("count").unwrap();
    let source_steps: Vec<&StepRecord> = outcome.steps_of(id("source")).collect();
    let first = source_steps[0];
    assert_eq!(first.disposition, Disposition::Backpressured { attempts: 1 });
    assert_eq!(first.pre, first.post);
    // The next attempt of the source is the same input again, before any other input.
    let resumed = source_steps[1];
    assert_eq!(resumed.message, first.message);
    assert_eq!(resumed.source, InputSource::Pending { attempt: 2 });
    assert_eq!(resumed.pre, first.pre);
    // Every input commits in the order it arrived, none is lost.
    let committed: Vec<_> = source_steps.iter().filter(|r| r.disposition == Disposition::Committed).map(|r| r.message).collect();
    let mut arrived: Vec<_> = source_steps.iter().filter(|r| r.source == InputSource::Mailbox).map(|r| r.message).collect();
    arrived.dedup();
    assert_eq!(committed, arrived);
    assert_eq!(committed.len(), 3);
    // While an input was held, no mailbox input of the source was attempted.
    let mut holding = false;
    for r in &source_steps {
        if holding {
            assert!(matches!(r.source, InputSource::Pending { .. }), "{r:#?}");
        }
        holding = r.disposition.is_held();
    }
    assert_eq!(outcome.program(id("target")).unwrap().state.word(count), 6);
    assert_eq!(outcome.policy.consumed, 0);
}

#[test]
fn backpressure_fills_the_mailbox_behind_the_held_input_and_pressure_goes_upstream() {
    // upstream (broadcaster) -> relay (volley) -> stalled (volley whose Hit needs a timer, and no
    // timer place exists). The stalled program holds its first input for the whole run.
    let (caster, v) = (verified(broadcaster()), verified(volley()));
    let (up, relay, stalled) = (id("upstream"), id("relay"), id("stalled"));
    let run = |sched: SchedulerConfig| {
        let config = scheduler(sched)
            .policy(AdmissionPolicy::Backpressure)
            .limits(big_bad_bool_runtime::RuntimeLimits {
                mailbox: 2,
                timers: 0,
                ..Default::default()
            });
        Runtime::builder(config)
            .program(up, ProgramInstance::new(&caster, broadcaster_table()))
            .program(relay, ProgramInstance::new(&v, rally_table()))
            .program(stalled, ProgramInstance::new(&v, stalled_table()))
            .route(Route::new(up, Port(0), relay))
            .route(Route::new(relay, Port(0), stalled))
            .inject(up, signal_message(START))
            .inject(up, signal_message(START))
            .inject_in_epoch(1, up, signal_message(START))
            .inject_in_epoch(1, up, signal_message(START))
            .inject_in_epoch(2, up, signal_message(START))
            .inject_in_epoch(2, up, signal_message(START))
            .build()
            .unwrap()
            .run()
    };
    let check = |outcome: &big_bad_bool_runtime::RunOutcome<_>| {
        let (s_up, s_relay, s_stalled) = (ProgramSlot(0), ProgramSlot(1), ProgramSlot(2));
        // The stalled program never committed and was refused for the timer place only.
        let stalled_steps: Vec<&StepRecord> = outcome.steps_of(stalled).collect();
        assert!(stalled_steps.iter().all(|r| r.admission != Admission::Admitted));
        assert_eq!(refused(stalled_steps[0]), [RuntimeRefusal::TooManyTimers { needed: 1, free: 0 }]);
        // Every later poll (woken by the relay's deliveries) attempts the same held input again.
        assert!(stalled_steps[1..].iter().all(|r| r.message == stalled_steps[0].message
            && matches!(r.source, InputSource::Pending { .. })));
        // Behind its held input, its mailbox filled: the relay's next send saw it full.
        let relay_refusal = outcome
            .steps_of(relay)
            .find(|r| r.admission != Admission::Admitted)
            .expect("the relay was refused");
        assert_eq!(refused(relay_refusal), [RuntimeRefusal::MailboxFull { to: s_stalled, needed: 1, free: 0 }]);
        assert!(relay_refusal.disposition.is_held());
        // And the relay's own mailbox filled behind its held input: upstream saw it full.
        let up_refusal = outcome
            .steps_of(up)
            .find(|r| r.admission != Admission::Admitted)
            .expect("upstream was refused");
        assert_eq!(refused(up_refusal), [RuntimeRefusal::MailboxFull { to: s_relay, needed: 1, free: 0 }]);
        assert!(up_refusal.disposition.is_held());
        // Nothing was lost silently: at shutdown every held and queued input is named.
        let finished: Vec<(ProgramSlot, usize, usize)> = outcome
            .log
            .iter()
            .filter_map(|e| match &e.kind {
                RuntimeEventKind::Finished { program, dropped, held, .. } => Some((*program, dropped.len(), held.len())),
                _ => None,
            })
            .collect();
        for slot in [s_up, s_relay, s_stalled] {
            let (_, _, held) = finished.iter().find(|(p, ..)| *p == slot).unwrap();
            assert_eq!(*held, 1, "{slot} holds one input at the end");
        }
        assert_eq!(outcome.policy.consumed, 0);
    };
    for ordering in WakeOrdering::ALL {
        for budget in BUDGETS {
            let sched = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
            let first = run(sched);
            check(&first);
            for _ in 0..4 {
                let again = run(sched);
                assert!(again.same_run(&first), "{} {budget}: {:?}", ordering.name(), again.diff(&first));
            }
        }
    }
}

#[test]
fn a_final_refusal_is_consumed_under_every_policy() {
    // No route from the source's port: final. Retry and Backpressure do not hold it.
    let caster = verified(broadcaster());
    let source = id("source");
    for policy in common::POLICIES {
        let outcome = Runtime::builder(fifo(|_| {}).policy(policy))
            .program(source, ProgramInstance::new(&caster, broadcaster_table()))
            .inject(source, signal_message(START))
            .inject(source, signal_message(START))
            .build()
            .unwrap()
            .run();
        let s = steps(&outcome);
        assert_eq!(s.len(), 2, "{}", policy.name());
        for r in s {
            assert_eq!(refused(r), [RuntimeRefusal::NoRoute { port: Port(0) }]);
            assert_eq!(r.disposition, Disposition::Consumed { final_refusal: true });
        }
    }
}

#[test]
fn a_program_can_name_its_own_policy() {
    let caster = verified(broadcaster());
    let target_program = verified(sink());
    let (source, target) = (id("source"), id("target"));
    let outcome = Runtime::builder(fifo(|l| l.mailbox = 1))
        .program(source, ProgramInstance::new(&caster, broadcaster_table()).policy(AdmissionPolicy::retry(1)))
        .program(target, ProgramInstance::new(&target_program, EffectTable::new()))
        .route(Route::new(source, Port(0), target))
        .inject(target, signal_message(HIT))
        .inject(source, signal_message(START))
        .build()
        .unwrap()
        .run();
    let shape: Vec<Disposition> = outcome.steps_of(source).map(|r| r.disposition).collect();
    assert_eq!(shape, [Disposition::Backlogged { len: 1 }, Disposition::Committed]);
    let [ActionRecord::Sent { deliveries, .. }] = outcome.steps_of(source).last().unwrap().actions.as_slice() else { panic!() };
    assert!(matches!(deliveries[0], Delivery::Delivered { to: ProgramSlot(1), .. }));
    assert!(outcome.steps_of(target).all(|r| r.origin == Origin::External || matches!(r.origin, Origin::Program(_))));
}
