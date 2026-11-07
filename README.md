# big-bad-bool-runtime

A deterministic runtime for many verified big-bad-bool programs on one
[scope](https://crates.io/crates/scope). Each program is one Scope task that owns its
`Stepper`; programs reach each other only through messages on declared routes. Every step is
prepared, admitted against every bound, and committed only when all of it fits.

Dependencies, all from crates.io: scope 0.3.2, big-bad-bool 0.1.2 with its `program` feature
(the program API is `big_bad_bool::program`, which is big-bad-bool-program 0.1.1), uuid_macros
0.1.0, and ob-serve-a-bowl 0.2.3 behind the `bowl` feature. It is the successor of the
registry-only experiment `attempts_scope_programs_registry`.

## The boundary

| Layer | Decides | Holds |
|---|---|---|
| program (`big_bad_bool::program`) | semantic legality: which contracts actualize, what is written, which effect words are emitted, what is refused | its state, inside its stepper |
| runtime (this crate) | admission and policy: whether a prepared step's runtime effects fit every bound, and what becomes of a refused input | routes, topology, accounting, timers, children, waiters, trace |
| scope | when: which task runs in which round (wake ordering, poll budget), the logical clock, bounded mailboxes, timers, cancellation, lifetimes | tasks and wakes |

## Usage

```rust
use big_bad_bool_runtime::fixtures::{rally, rally_table, signal_message, verified, HIT};
use big_bad_bool_runtime::{Port, ProgramId, ProgramInstance, Route, Runtime, RuntimeConfig};

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
assert_eq!(outcome.steps, 9);
```

A `ProgramInstance` is a verified program, its `EffectTable` (data: `(effect dimension, word)`
to `RuntimeEffect`s, resolved against the spec at `build`), an optional step observer and an
optional admission policy of its own. `build` makes each program's stepper; the run moves it
into the program's Scope task. No API returns a stepper or `&mut` state; the finished run hands
back final states (`RunOutcome::programs`).

`RuntimeEffect`: `Send { port, message }`, `Timer { after, message }` (Scope's logical clock),
`StartChild { key, child }`, `CancelChild { key }` (Scope's cancellation), `External { tag,
word }`, `Trace { tag }`. There is no wall-clock timer.

## One step

1. `prepare`: results, effects and refusals, with nothing changed.
2. admission: every runtime effect of the step at once, against effects per step, routes,
   deliveries per step, each receiver's free mailbox places (one place of every mailbox is kept
   for the shutdown word), free timer places, free child places, child keys, the step budget.
3. all of it fits: `commit` exactly that preparation, then act. Anything does not fit: nothing is
   committed and nothing is performed; the trace gets one `RuntimeEvent` with every reason and
   `pre == post`.

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
`Timers`, `Children`). The task that frees the place (a receive, a commit of a held input or a
finish for a mailbox; a delivered timer; an ended child) wakes every waiting task, in the order
they began waiting, through `Cx::wake_with(.., WakeReason::Manual)`, and records a `Released`
event. A held program is also polled on every other wake it gets (a new message in its mailbox),
and attempts its held input then. Scope has no wake for "a bounded mailbox has room again"; this
waiter list is the substitute (docs/MISSING_APIS.md).

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

## Determinism

With logical timers, a run is a function of the wake sequence, Scope's policy and the
tick-advancement rule (one tick per scheduler round). The same configuration and inputs give the
same `RuntimeEvent`s (sequence, tick, program, message, pre and post digests, preparation,
admission and reasons, actions, routing, timers, children, policy dispositions), the same Scope
scheduler events, the same external words and the same final states: `RunOutcome::same_run`,
and `RunOutcome::diff` for the first difference. Nothing in the crate reads the wall clock,
randomness or the environment; real-time timers are outside replay and outside this crate.

Ids are uuid_macros v5 newtypes under one namespace per kind (`ProgramId::from_name`,
`RouteId::of`, `TimerId::of`, `RunId::of_topology`); no random id is made. Inside a run,
programs are dense `ProgramSlot`s (registration order).

## Observers

A `RuntimeObserver` sees every `RuntimeEvent` by shared reference and returns nothing
(`EventCollector` keeps them). The `bowl` feature adds `BowlRuntimeObserver`, which publishes
every event to an ob-serve-a-bowl runtime (prepare, refuse, admit, commit, route, timer, cancel,
policy), and turns on big-bad-bool-program's `BowlObserver` for program step events. A refused
step publishes `Refused` and never `Committed`. Runs with and without observers are the same run.

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
cargo +1.88 test --all-features    # plus the bowl observers (ob-serve-a-bowl needs Rust 1.88)
cargo +1.85 test                   # minimum Rust without bowl
cargo +nightly test --doc          # the compile-fail cases with their error codes checked
cargo bench --bench runtime        # the measurements (about 90 s), docs/MEASUREMENTS.md
```

docs/SHARDING.md, docs/NO_STD.md and docs/MISSING_APIS.md say what is not here and why.

## License

MIT OR Apache-2.0.
