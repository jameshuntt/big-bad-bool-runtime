//! Many senders, few places: Scope's capacity waiters wake one waiting sender per freed place and
//! hold that place for it, under every ordering and poll budget; the two waits Scope cannot see
//! (a `Backpressure` input holding a place, a step needing two places) still commit everything.

mod common;

use big_bad_bool::program::NoObserver;
use big_bad_bool_runtime::{
    AdmissionPolicy, Disposition, EffectTable, Port, ProgramInstance, Route, RunOutcome, Runtime,
    RuntimeEventKind, RuntimeLimits, SchedEvent,
};
use common::fixtures::{
    START, broadcaster, broadcaster_table, signal_message, sink, twice_table, verified, volley,
};
use common::{BUDGETS, id, scheduler};
use scope::{SchedulerConfig, WakeOrdering, WakeReason};

const POLICIES: [AdmissionPolicy; 2] = [AdmissionPolicy::retry(16), AdmissionPolicy::Backpressure];

/// `casters` broadcasters, `starts` starts each, into one sink (or a relay before it) with
/// `mailbox` places: the runtime's `policy/fan_in_64x16` shape when 64, 16, 16.
fn fan_in(
    casters: u32,
    starts: u32,
    mailbox: usize,
    policy: AdmissionPolicy,
    sched: SchedulerConfig,
    relay_twice: bool,
) -> RunOutcome<NoObserver> {
    let (caster, s, v) = (
        verified(broadcaster()),
        verified(sink()),
        verified(volley()),
    );
    let config = scheduler(sched).policy(policy).limits(RuntimeLimits {
        mailbox,
        retry_backlog: 64,
        ..RuntimeLimits::default()
    });
    let target = id("sink");
    let mut b =
        Runtime::builder(config).program(target, ProgramInstance::new(&s, EffectTable::new()));
    let first = if relay_twice {
        let relay = id("relay");
        b = b
            .program(relay, ProgramInstance::new(&v, twice_table()))
            .route(Route::new(relay, Port(0), target));
        relay
    } else {
        target
    };
    for i in 0..casters {
        let c = id(&format!("caster {i}"));
        b = b
            .program(c, ProgramInstance::new(&caster, broadcaster_table()))
            .route(Route::new(c, Port(0), first));
        for _ in 0..starts {
            b = b.inject(c, signal_message(START));
        }
    }
    b.build().unwrap().run()
}

/// Committed steps of every program but the sink (slot 0): the casters, and the relay if any.
fn caster_commits(o: &RunOutcome<NoObserver>) -> u64 {
    o.step_records()
        .filter(|r| r.disposition == Disposition::Committed && r.program.0 > 0)
        .count() as u64
}

fn capacity_wakes(o: &RunOutcome<NoObserver>) -> u64 {
    o.scheduler
        .iter()
        .filter(|e| matches!(e, SchedEvent::Wake(_, WakeReason::Capacity)))
        .count() as u64
}

#[test]
fn sixty_four_senders_into_sixteen_places_wake_one_sender_per_place_and_commit_everything() {
    for policy in POLICIES {
        let o = fan_in(64, 16, 16, policy, SchedulerConfig::default(), false);
        let commits = caster_commits(&o);
        assert_eq!(commits, 1024, "{}: every start commits", policy.name());
        assert_eq!(o.steps, 2048, "and the sink takes every hit");
        // one capacity wake per freed place a sender waited for, never one per waiter
        let wakes = capacity_wakes(&o);
        assert!(
            wakes <= commits,
            "{}: {wakes} wakes for {commits} commits",
            policy.name()
        );
        // without the runtime's own list (nothing is held, every step needs one place)
        assert!(
            !o.log
                .iter()
                .any(|e| matches!(e.kind, RuntimeEventKind::Released { .. }))
        );
        // refused attempts per committed step: 11.41 (Retry) and 14.42 (Backpressure) in 0.1.0
        let per_step = o.refused as f64 / o.steps as f64;
        let bound = if policy == AdmissionPolicy::Backpressure {
            0.5
        } else {
            1.5
        };
        assert!(
            per_step <= bound,
            "{}: {per_step} refused per committed step",
            policy.name()
        );
    }
}

#[test]
fn contention_is_the_same_run_every_time_under_every_ordering_and_budget() {
    for policy in POLICIES {
        for ordering in WakeOrdering::ALL {
            for budget in BUDGETS {
                let sched = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
                // a caster's starts wait in its own mailbox, of the same two places
                let first = fan_in(12, 2, 2, policy, sched, false);
                assert_eq!(caster_commits(&first), 24, "{} {budget}", ordering.name());
                let again = fan_in(12, 2, 2, policy, sched, false);
                assert!(
                    again.same_run(&first),
                    "{} {} {budget}: {:?}",
                    policy.name(),
                    ordering.name(),
                    again.diff(&first)
                );
            }
        }
    }
}

#[test]
fn a_held_input_and_a_step_needing_two_places_still_commit_everything() {
    // casters -> relay (each hit goes out twice: two places in the sink) -> sink. The relay holds
    // an input whenever the sink has fewer than two places, and its own mailbox fills behind it:
    // its senders wait on the runtime's list, which the relay wakes when it commits or takes.
    for policy in POLICIES {
        for ordering in WakeOrdering::ALL {
            let sched = SchedulerConfig::with_ordering(ordering);
            let o = fan_in(6, 2, 2, policy, sched, true);
            // the casters' 12 starts and the relay's 12 hits
            assert_eq!(
                caster_commits(&o),
                12 + 12,
                "{} {}",
                policy.name(),
                ordering.name()
            );
            let held: usize = o
                .log
                .iter()
                .filter_map(|e| match &e.kind {
                    RuntimeEventKind::Finished { held, .. } => Some(held.len()),
                    _ => None,
                })
                .sum();
            assert_eq!(
                held,
                0,
                "{} {}: nothing held at the end",
                policy.name(),
                ordering.name()
            );
            assert!(
                o.log
                    .iter()
                    .any(|e| matches!(e.kind, RuntimeEventKind::Released { .. })),
                "the runtime's list was used"
            );
        }
    }
}
