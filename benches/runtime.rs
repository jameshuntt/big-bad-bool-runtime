// criterion_group! generates an undocumented function.
#![allow(missing_docs)]

//! Measurements, not optimizations. `cargo bench --bench runtime`. The registry experiment's
//! shapes (same names, same sizes), plus admission refusal and the three admission policies.

use std::hint::black_box;
use std::time::Duration;

use big_bad_bool::program::{NoObserver, Observation, Stepper, VerifiedProgram};
use big_bad_bool_runtime::fixtures::{
    HIT, START, broadcaster, broadcaster_table, metronome, metronome_table, rally, rally_table,
    signal_message, sink, verified, volley,
};
use big_bad_bool_runtime::{
    AdmissionPolicy, EffectTable, Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig,
    RuntimeLimits, TraceLevel,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use scope::{
    BoundedMailbox, BoundedSender, Cx, Mailbox, MailboxSender, Scope, ScopedTask, TaskPoll,
};

const VOLLEY_STEPS: u64 = 10_000;

fn off() -> RuntimeConfig {
    RuntimeConfig::default().trace(TraceLevel::Off)
}

/// Program ids made once, outside every timed loop.
fn id(n: u32) -> ProgramId {
    static IDS: std::sync::OnceLock<Vec<ProgramId>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        (0..2100)
            .map(|n| ProgramId::from_name(&format!("p{n}")))
            .collect()
    })[n as usize]
}

fn volley_run(
    p: &VerifiedProgram,
    idle_program: &VerifiedProgram,
    steps: u64,
    idle: u32,
    trace: TraceLevel,
) -> u64 {
    let limits = RuntimeLimits {
        steps,
        ..RuntimeLimits::default()
    };
    let (a, b) = (id(0), id(1));
    let mut builder = Runtime::builder(RuntimeConfig::default().limits(limits).trace(trace))
        .program(a, ProgramInstance::new(p, rally_table()))
        .program(b, ProgramInstance::new(p, rally_table()))
        .route(Route::new(a, Port(0), b))
        .route(Route::new(b, Port(0), a))
        .inject(a, signal_message(HIT));
    for i in 0..idle {
        builder = builder.program(
            id(10 + i),
            ProgramInstance::new(idle_program, EffectTable::new()),
        );
    }
    builder.build().unwrap().run().steps
}

fn steps(c: &mut Criterion) {
    let _ = id(0);
    let p = verified(volley());
    let idle = verified(sink());
    let signal = p.spec().dimension_id("signal").unwrap();
    let mut g = c.benchmark_group("steps");
    g.throughput(Throughput::Elements(VOLLEY_STEPS));
    g.bench_function("bare_stepper_push", |bch| {
        bch.iter(|| {
            let mut st = Stepper::new(&p, NoObserver);
            for i in 0..VOLLEY_STEPS as u32 {
                black_box(st.push(Observation::new(i).with(signal, HIT)));
            }
        })
    });
    g.bench_function("prepare_commit", |bch| {
        bch.iter(|| {
            let mut st = Stepper::new(&p, NoObserver);
            for i in 0..VOLLEY_STEPS as u32 {
                let prepared = st.prepare(Observation::new(i).with(signal, HIT));
                black_box(prepared.effects().count());
                black_box(st.commit(prepared).unwrap());
            }
        })
    });
    // One sink, every message injected: prepare, admit (nothing to admit), commit on the runtime.
    let s = verified(sink());
    g.bench_function("runtime_prepare_admit_commit_unrouted", |bch| {
        bch.iter(|| {
            let config = off().limits(RuntimeLimits {
                mailbox: VOLLEY_STEPS as usize,
                ..RuntimeLimits::default()
            });
            let mut b = Runtime::builder(config)
                .program(id(0), ProgramInstance::new(&s, EffectTable::new()));
            for _ in 0..VOLLEY_STEPS {
                b = b.inject(id(0), signal_message(HIT));
            }
            black_box(b.build().unwrap().run().steps)
        })
    });
    g.bench_function("runtime_volley_routed", |bch| {
        bch.iter(|| black_box(volley_run(&p, &idle, VOLLEY_STEPS, 0, TraceLevel::Off)))
    });
    g.bench_function("runtime_volley_routed_full_trace", |bch| {
        bch.iter(|| black_box(volley_run(&p, &idle, VOLLEY_STEPS, 0, TraceLevel::Full)))
    });
    g.finish();
}

fn admission(c: &mut Criterion) {
    // One volley, 10,000 injected hits; each prepared Hit asks for a Send on a port with no
    // route: prepare, admission refuses (final), the input is consumed. Nothing commits.
    let p = verified(volley());
    let mut g = c.benchmark_group("admission");
    g.throughput(Throughput::Elements(VOLLEY_STEPS));
    g.bench_function("refusal_prepare_admit_refuse", |bch| {
        bch.iter(|| {
            let config = off().limits(RuntimeLimits {
                mailbox: VOLLEY_STEPS as usize,
                ..RuntimeLimits::default()
            });
            let mut b =
                Runtime::builder(config).program(id(0), ProgramInstance::new(&p, rally_table()));
            for _ in 0..VOLLEY_STEPS {
                b = b.inject(id(0), signal_message(HIT));
            }
            let out = b.build().unwrap().run();
            assert_eq!(out.steps, 0);
            black_box(out.refused)
        })
    });
    g.finish();
}

/// 64 casters, 16 starts each, into one sink with 16 places: the sink takes one message per
/// poll while 64 senders want a place, so admission refuses often.
fn fan_in(c: &VerifiedProgram, s: &VerifiedProgram, policy: AdmissionPolicy) -> (u64, u64) {
    let config = off().policy(policy).limits(RuntimeLimits {
        mailbox: 16,
        ..RuntimeLimits::default()
    });
    let target = id(0);
    let mut b =
        Runtime::builder(config).program(target, ProgramInstance::new(s, EffectTable::new()));
    for i in 1..=64 {
        b = b
            .program(id(i), ProgramInstance::new(c, broadcaster_table()))
            .route(Route::new(id(i), Port(0), target));
        for _ in 0..16 {
            b = b.inject(id(i), signal_message(START));
        }
    }
    let out = b.build().unwrap().run();
    (out.steps, out.refused)
}

fn policies(c: &mut Criterion) {
    let (caster, s) = (verified(broadcaster()), verified(sink()));
    let mut g = c.benchmark_group("policy");
    for (name, policy) in [
        ("consume", AdmissionPolicy::Consume),
        ("retry", AdmissionPolicy::retry(16)),
        ("backpressure", AdmissionPolicy::Backpressure),
    ] {
        let (steps, refused) = fan_in(&caster, &s, policy);
        eprintln!(
            "policy/fan_in_64x16/{name}: {steps} committed steps, {refused} refused attempts"
        );
        g.throughput(Throughput::Elements(steps));
        g.bench_function(BenchmarkId::new("fan_in_64x16", name), |bch| {
            bch.iter(|| black_box(fan_in(&caster, &s, policy)))
        });
    }
    g.finish();
}

fn timers(c: &mut Criterion) {
    let m = verified(metronome());
    let mut g = c.benchmark_group("timer");
    g.throughput(Throughput::Elements(1_000));
    g.bench_function("logical_metronome_1000_beats", |bch| {
        bch.iter(|| {
            let config = off().limits(RuntimeLimits {
                steps: 1_000,
                ..RuntimeLimits::default()
            });
            let out = Runtime::builder(config)
                .program(id(0), ProgramInstance::new(&m, metronome_table()))
                .inject(id(0), signal_message(START))
                .build()
                .unwrap()
                .run();
            black_box(out.steps)
        })
    });
    g.finish();
}

/// Scope alone: two mailbox tasks passing a counter, the wake path every routed message takes.
struct Bouncer<'d> {
    inbox: Mailbox<u64>,
    peer: &'d std::cell::OnceCell<MailboxSender<u64>>,
}

impl<'env, 'd: 'env> ScopedTask<'env, u64> for Bouncer<'d> {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        while let Some(n) = self.inbox.try_recv() {
            *cx.resources += 1;
            if n > 0 {
                self.peer.get().unwrap().send(n - 1);
            }
        }
        TaskPoll::Pending
    }
}

/// The same over Scope's bounded mailbox with `try_send`.
struct BoundedBouncer<'d> {
    inbox: BoundedMailbox<u64>,
    peer: &'d std::cell::OnceCell<BoundedSender<u64>>,
}

impl<'env, 'd: 'env> ScopedTask<'env, u64> for BoundedBouncer<'d> {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        while let Some(n) = self.inbox.try_recv() {
            *cx.resources += 1;
            if n > 0 {
                let _ = self.peer.get().unwrap().try_send(n - 1);
            }
        }
        TaskPoll::Pending
    }
}

fn scope_wakeups(c: &mut Criterion) {
    let mut g = c.benchmark_group("scheduler");
    g.throughput(Throughput::Elements(VOLLEY_STEPS));
    g.bench_function("scope_mailbox_bounce", |bch| {
        bch.iter(|| {
            let mut total = 0u64;
            let (to_a, to_b) = (std::cell::OnceCell::new(), std::cell::OnceCell::new());
            {
                let mut scope = Scope::new(&mut total);
                let (_, ta) = scope.spawn_with_mailbox("a", |inbox| Bouncer { inbox, peer: &to_b });
                let (_, tb) = scope.spawn_with_mailbox("b", |inbox| Bouncer { inbox, peer: &to_a });
                ta.send(VOLLEY_STEPS - 1);
                let _ = to_a.set(ta);
                let _ = to_b.set(tb);
                scope.run();
            }
            black_box(total)
        })
    });
    g.bench_function("scope_bounded_bounce", |bch| {
        bch.iter(|| {
            let mut total = 0u64;
            let (to_a, to_b) = (std::cell::OnceCell::new(), std::cell::OnceCell::new());
            {
                let mut scope = Scope::new(&mut total);
                let (_, ta) = scope.spawn_with_bounded_mailbox("a", 4, |inbox| BoundedBouncer {
                    inbox,
                    peer: &to_b,
                });
                let (_, tb) = scope.spawn_with_bounded_mailbox("b", 4, |inbox| BoundedBouncer {
                    inbox,
                    peer: &to_a,
                });
                let _ = ta.try_send(VOLLEY_STEPS - 1);
                let _ = to_a.set(ta);
                let _ = to_b.set(tb);
                scope.run();
            }
            black_box(total)
        })
    });
    g.finish();
}

fn instances(c: &mut Criterion) {
    let p = verified(sink());
    let mut g = c.benchmark_group("instances");
    for n in [1u32, 10, 100, 1000] {
        g.throughput(Throughput::Elements(u64::from(n) * 4));
        g.bench_with_input(
            BenchmarkId::new("independent_sinks_4_hits", n),
            &n,
            |bch, &n| {
                bch.iter(|| {
                    let mut b = Runtime::builder(off());
                    for i in 0..n {
                        b = b.program(id(i), ProgramInstance::new(&p, EffectTable::new()));
                        for _ in 0..4 {
                            b = b.inject(id(i), signal_message(HIT));
                        }
                    }
                    black_box(b.build().unwrap().run().steps)
                })
            },
        );
    }
    let r = verified(rally());
    for pairs in [1u32, 5, 50, 500] {
        g.throughput(Throughput::Elements(u64::from(pairs) * 9));
        g.bench_with_input(
            BenchmarkId::new("rally_pairs_instances", pairs * 2),
            &pairs,
            |bch, &pairs| {
                bch.iter(|| {
                    let mut b = Runtime::builder(off());
                    for pair in 0..pairs {
                        let (x, y) = (id(pair * 2), id(pair * 2 + 1));
                        b = b
                            .program(x, ProgramInstance::new(&r, rally_table()))
                            .program(y, ProgramInstance::new(&r, rally_table()))
                            .route(Route::new(x, Port(0), y))
                            .route(Route::new(y, Port(0), x))
                            .inject(x, signal_message(HIT));
                    }
                    black_box(b.build().unwrap().run().steps)
                })
            },
        );
    }
    g.finish();
}

fn fan_out(c: &mut Criterion) {
    let caster = verified(broadcaster());
    let s = verified(sink());
    let mut g = c.benchmark_group("fan_out");
    for k in [1u32, 8, 64] {
        g.throughput(Throughput::Elements(u64::from(k)));
        g.bench_with_input(
            BenchmarkId::new("one_commit_to_k_sinks", k),
            &k,
            |bch, &k| {
                bch.iter(|| {
                    let hub = id(0);
                    let mut b = Runtime::builder(off())
                        .program(hub, ProgramInstance::new(&caster, broadcaster_table()));
                    for i in 1..=k {
                        b = b
                            .program(id(i), ProgramInstance::new(&s, EffectTable::new()))
                            .route(Route::new(hub, Port(0), id(i)));
                    }
                    black_box(
                        b.inject(hub, signal_message(START))
                            .build()
                            .unwrap()
                            .run()
                            .steps,
                    )
                })
            },
        );
    }
    g.finish();
}

fn trace_overhead(c: &mut Criterion) {
    let r = verified(rally());
    let mut g = c.benchmark_group("trace");
    g.throughput(Throughput::Elements(100 * 9));
    for (name, level) in [("off", TraceLevel::Off), ("full", TraceLevel::Full)] {
        g.bench_function(BenchmarkId::new("rally_100_pairs", name), |bch| {
            bch.iter(|| {
                let mut b = Runtime::builder(RuntimeConfig::default().trace(level));
                for pair in 0..100u32 {
                    let (x, y) = (id(pair * 2), id(pair * 2 + 1));
                    b = b
                        .program(x, ProgramInstance::new(&r, rally_table()))
                        .program(y, ProgramInstance::new(&r, rally_table()))
                        .route(Route::new(x, Port(0), y))
                        .route(Route::new(y, Port(0), x))
                        .inject(x, signal_message(HIT));
                }
                black_box(b.build().unwrap().run().steps)
            })
        });
    }
    g.finish();
}

fn idle_vs_active(c: &mut Criterion) {
    let p = verified(volley());
    let s = verified(sink());
    let mut g = c.benchmark_group("idle");
    g.throughput(Throughput::Elements(2_000));
    for idle in [0u32, 100, 1000] {
        g.bench_with_input(
            BenchmarkId::new("volley_2000_steps_with_idle", idle),
            &idle,
            |bch, &idle| bch.iter(|| black_box(volley_run(&p, &s, 2_000, idle, TraceLevel::Off))),
        );
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2));
    targets = steps, admission, policies, timers, scope_wakeups, instances, fan_out, trace_overhead, idle_vs_active
}
criterion_main!(benches);
