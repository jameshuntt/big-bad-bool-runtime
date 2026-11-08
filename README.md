# big-bad-bool-runtime

A deterministic runtime for many verified big-bad-bool programs on one
[scope](https://crates.io/crates/scope). Each program is one Scope task that owns its
`Stepper`; programs reach each other only through messages on declared routes. Every step is
prepared, admitted against every bound, and committed only when all of it fits.

```toml
[dependencies]
big-bad-bool = { version = "0.1.2", features = ["program"] }  # the programs
big-bad-bool-runtime = "0.2"
```

Dependencies, all from crates.io: scope 0.4.0, big-bad-bool 0.1.2 with its `program` feature
(the program API is `big_bad_bool::program`, which is big-bad-bool-program 0.1.1), uuid_macros
0.1.0, and ob-serve-a-bowl 0.2.3 behind the `bowl` feature.

## Three layers

| Layer | Decides | Holds |
|---|---|---|
| program (`big_bad_bool::program`) | semantic authority: which contracts actualize, what is written, which effect words are emitted, what is refused | its state, inside its stepper |
| runtime (this crate) | routing, admission and policy: where an effect word goes, whether a prepared step's runtime effects fit every bound, and what becomes of a refused input | routes, topology, accounting, timers, children, waiters, trace |
| scope | scheduling, lifetime and logical time: which task runs in which round (wake ordering, poll budget), the logical clock, bounded mailboxes, timers, cancellation | tasks and wakes |

## One step: prepare, preflight, commit, perform

1. **prepare**: `Stepper::prepare` gives the step's results, effect words and refusals, with
   nothing changed.
2. **preflight all actions**: the program's `EffectTable` (data, resolved against the spec at
   `build`) maps the effect words to `RuntimeEffect`s, and admission checks every one of them at
   once: effects per step, routes, deliveries per step, each receiver's free mailbox places (a place
   Scope holds for a woken waiter counts as taken, except for that waiter), free timer places, free child places,
   child keys, the step budget.
3. **commit**: when all of it fits, `Stepper::commit` applies exactly that preparation.
4. **perform**: then the runtime acts: sends, Scope logical timers, Scope child tasks and their
   cancellation, external words, trace marks.

When anything does not fit, nothing is committed and nothing is performed; the trace gets one
`RuntimeEvent` with every reason and `pre == post`.

`RuntimeEffect`: `Send { port, message }`, `Timer { after, message }` (Scope's logical clock),
`StartChild { key, child }`, `CancelChild { key }` (Scope's cancellation), `External { tag,
word }`, `Trace { tag }`. There is no wall-clock timer.

## What holds

- **No admission, no commit.** A step whose runtime effects do not all fit changes no state and
  performs no action.
- **State is isolated per Scope task.** `build` makes each program's stepper and the run moves it
  into the program's task. No API returns a stepper or `&mut` state; the finished run hands back
  final states (`RunOutcome::programs`). Compile-fail doctests in the crate docs check it.
- **Programs meet only through messages and effects.** A program reaches another only by a
  `Send` on a declared `Route`; there is no shared state between programs.
- **Scope schedules.** Scheduling is Scope's deterministic cooperative scheduling, and time is
  its logical clock (one tick per scheduler round).
- **Policies.** A refused input is consumed (`Consume`), retried from a bounded backlog
  (`Retry`), or held while the program takes nothing new (`Backpressure`); per runtime or per
  program.
- **Bowl is observation only.** The `bowl` feature publishes events to ob-serve-a-bowl;
  observers see events by shared reference and return nothing, and runs with and without them
  are the same run.
- **Typed ids.** `ProgramId`, `RouteId`, `RunId` and `TimerId` are uuid_macros v5 newtypes under
  one namespace per kind (`ProgramId::from_name`, `RouteId::of`, `TimerId::of`,
  `RunId::of_topology`); no random id is made. Inside a run, programs are dense `ProgramSlot`s
  in registration order.
- **Determinism.** The same inputs, topology, configuration and policy give the same
  `RuntimeEvent`s (sequence, tick, program, message, pre and post digests, preparation,
  admission and reasons, actions, routing, timers, children, policy dispositions), the same
  Scope scheduler events, the same external words and the same final states, within the
  semantics written here: `RunOutcome::same_run`, and `RunOutcome::diff` for the first
  difference. Nothing in the crate reads the wall clock, randomness or the environment.

## Examples

The programs in these examples are the test fixtures, `tests/common/fixtures.rs`: small
verified programs with one evidence dimension `signal`. CI runs every example.

**Ping-pong.** [`examples/ping_pong.rs`](examples/ping_pong.rs), in part: two rally programs
return a hit to each other until each has counted four.

```rust
let rally = verified(rally());
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
```

Nine steps commit; the ninth emits one external word.

**A refusal under Retry and Backpressure.** [`examples/policies.rs`](examples/policies.rs), in
part: the source sends to a target whose mailbox (two places) is full.

```rust
let config = RuntimeConfig::with_scheduler(SchedulerConfig::with_ordering(WakeOrdering::Fifo))
    .policy(policy) // Consume, AdmissionPolicy::retry(4) or Backpressure
    .limits(RuntimeLimits { mailbox: 2, ..RuntimeLimits::default() });
let outcome = Runtime::builder(config)
    .program(source, ProgramInstance::new(&caster, broadcaster_table()))
    .program(target, ProgramInstance::new(&s, EffectTable::new()))
    .route(Route::new(source, Port(0), target))
    .inject(target, signal_message(HIT))
    .inject(target, signal_message(HIT))
    .inject(source, signal_message(START))
    .inject(source, signal_message(START))
    .build()
    .expect("within bounds")
    .run();
```

```text
backpressure:
  #5   t1 m2 Mailbox Refused([MailboxFull { to: ProgramSlot(1), needed: 1, free: 0 }]) Backpressured { attempts: 1 }
  #7   t1 released Mailbox(ProgramSlot(1)), woke [TaskId { slot: 0, generation: 0 }]
  #8   t2 m2 Pending { attempt: 2 } Admitted Committed
```

The refused step commits nothing; when the target takes a message, the held input is tried
again and commits. Under `Consume` the same input is consumed instead.

**A logical timer.** [`examples/logical_timer.rs`](examples/logical_timer.rs), in part: a
commit arms a Scope logical timer, which delivers its message five ticks later.

```rust
let run = || {
    Runtime::builder(RuntimeConfig::default())
        .program(id, ProgramInstance::new(&p, alarm_table()))
        .inject(id, signal_message(START))
        .build()
        .expect("within bounds")
        .run()
};
let first = run();
let same = (0..10).all(|_| run().same_run(&first));
```

```text
#2 t1 step m0 Admitted [TimerArmed { timer: TimerId(f5aee786-2c62-57db-b38f-c9441a8bed98), message: MessageId(1), after: 5 }]
#3 t7 timer f5aee786-2c62-57db-b38f-c9441a8bed98 fired: Delivered { to: ProgramSlot(0), message: MessageId(1) }
#4 t8 step m1 Admitted [External { tag: 2, word: 4 }]
ten more runs are the same run: true
```

## Admission policies

A refusal is *final* when no later event of the run can make the same step fit (too many
effects or deliveries, no route, a finished receiver, an unknown child, the step budget). A final
refusal consumes the input under every policy. The others (a full mailbox, no timer or child
place, a child key in use) wait for a place the run frees.

- `Consume`: the input is consumed; nothing is committed. A host resends in a later epoch.
- `Retry { backlog, max_attempts }`: the input goes to the back of a backlog of at most
  `backlog` inputs (a full backlog drops it with a `BacklogFull` event). At the start of each
  poll, before any new mailbox message, the backlog is retried oldest first; each retry is a new
  prepare and admission; retrying stops at the first entry refused again, which keeps its place.
  An entry refused `max_attempts` times is dropped (`Exhausted`). Then the poll takes at most one
  new mailbox message, so newer input can commit before older held input.
- `Backpressure`: the input is held and the program takes no new mailbox message until it
  commits or is refused finally. The held input keeps one place of the program's mailbox, so the
  mailbox fills behind it and senders are refused `MailboxFull` by their own admission. Input
  order is kept.

Timer deliveries and child ticks are mailbox messages, so held inputs go before them too.

**Wake rule.** A held input waits for the place its refusal named (`Resource::Mailbox`,
`Timers`, `Children`). A mailbox place is Scope's to give: the task registers in the receiver's
bounded mailbox (`BoundedSender::wake_when_free`, scope 0.4), in the order senders began waiting.
A receive holds each freed place for the oldest waiter and wakes only it
(`WakeReason::Capacity`); the woken task fills it (`try_send_from`), and nobody else can take it.
The task gives up a registration (and a held place, which goes to the next waiter) when its
refusals no longer name that mailbox, and when it ends. Two waits stay on the runtime's own list
for the mailbox, because Scope cannot see the place they need: the place an input `Backpressure`
holds outside its mailbox, and a step that needs more than one place in one mailbox. The program
that frees such a place (a receive, a commit or drop of its held input, its finish) wakes that
list, in order, and records a `Released` event. Timer and child places keep runtime lists, woken
by the delivered timer or the ended child (`WakeReason::Manual`, `Released`). A held program is
also polled on every other wake it gets (a new message in its mailbox), and attempts its held
input then.

A timer whose tick comes while its program's mailbox is full waits for a place
(`TimerWaiting`) and keeps its timer place until it delivers. A child tick into a full mailbox is
refused and recorded.

## Bounds

All in `RuntimeConfig::limits`, each with one deterministic result:

| Bound | Default | Reached |
|---|---|---|
| `programs` | 4096 | `BuildError::TooManyPrograms` |
| `routes` | 65536 | `BuildError::TooManyRoutes` |
| `fanout` (routes per port) | 64 | `BuildError::FanOut` |
| `mailbox` | 64 | `MailboxFull` (step), refused injection or child tick, waiting timer |
| `retry_backlog` | 64 | `BuildError::Backlog`; at run time a full backlog is `Disposition::BacklogFull` |
| `deliveries_per_step` | 64 | `RuntimeRefusal::FanOut` |
| `effects_per_step` | 16 | `RuntimeRefusal::EffectsPerStep` |
| `timers` | 256 | `RuntimeRefusal::TooManyTimers` |
| `children` | 256 | `RuntimeRefusal::TooManyChildren` |
| `steps` | 1,000,000 | `RuntimeRefusal::StepBudget` for every later input |
| `trace_events` | 1,000,000 | oldest event dropped and counted (`trace_dropped`, `scheduler_dropped`); sequence numbers keep the gap |
| `child_cancel_ticks` | 2 | Scope hard-stops a cancelled child after this many rounds |

## Observers

A `RuntimeObserver` sees every `RuntimeEvent` by shared reference and returns nothing
(`EventCollector` keeps them). The `bowl` feature adds `BowlRuntimeObserver`, which publishes
every event to an ob-serve-a-bowl runtime (prepare, refuse, admit, commit, route, timer, cancel,
policy), and turns on big-bad-bool-program's `BowlObserver` for program step events. A refused
step publishes `Refused` and never `Committed`.

## Limitations

- **Fan-in wakes, two cases left.** Since scope 0.4 a freed mailbox place wakes one waiting
  sender, the oldest, and is held for it: with 64 senders and one receiver, 1.41 (`Retry`) and
  0.49 (`Backpressure`) refused attempts per committed step, against 11.4 and 14.4 in 0.1.0
  (docs/MEASUREMENTS.md). Two waits still go through the runtime's own list, which wakes every
  task on it when the place may have come free: senders to a program whose `Backpressure` input
  holds a place (Scope cannot see that place; docs/MISSING_APIS.md item 2), and steps that need
  more than one place in one mailbox (they give up a held place and wait for unheld ones, so
  under steady single-place contention they can wait long). It costs time, not correctness.
- One runtime runs on one Scope; [docs/SHARDING.md](docs/SHARDING.md) says why there is no
  sharding here, [docs/NO_STD.md](docs/NO_STD.md) what `no_std` would need, and
  [docs/MEASUREMENTS.md](docs/MEASUREMENTS.md) has the measurements.

## Rust versions

Rust 1.85 or newer (`rust-version`). The `bowl` feature needs 1.88, the rust-version of
ob-serve-a-bowl 0.2.3.

## Run it

```sh
cargo run --example ping_pong      # two rallies, every event with its tick and admission
cargo run --example logical_timer  # a commit arms a logical timer; ten more runs are the same run
cargo run --example cancellation   # a commit starts a Scope child, a later commit cancels it
cargo run --example fan_out        # one commit, eight receivers, admitted as one
cargo run --example mailbox_full   # Consume: refused before commit, resent in epoch 1, commits
cargo run --example policies       # one refused send under Consume, Retry and Backpressure
cargo run --release --example contention   # the determinism table: 5 x 3 x 3 x 10 runs

cargo test                         # every scenario
cargo +1.88 test --all-features    # plus the bowl observers
cargo +nightly test --doc          # the compile-fail cases with their error codes checked
cargo bench --bench runtime        # the measurements (about 90 s), docs/MEASUREMENTS.md
```

## License

MIT OR Apache-2.0.
