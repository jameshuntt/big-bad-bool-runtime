# Changelog

## Unreleased

First version, as the successor of the registry-only experiment
`attempts_scope_programs_registry` (8081142), on crates.io dependencies only: scope 0.3.2,
big-bad-bool 0.1.2 with `program` (big-bad-bool-program 0.1.1), uuid_macros 0.1.0, and
ob-serve-a-bowl 0.2.3 behind `bowl`.

Kept from the experiment: one Scope task per program owning its stepper, effect tables as data
resolved at `build`, admission of every runtime effect of a step before its commit (the
mailbox, fan-out, timer, child, effects-per-step and step-budget checks are the experiment's),
Scope logical timers, Scope cancellation of children, bounded mailboxes with one place kept for
shutdown, the fixtures, and the scenario tests.

Changed:

- Names: `Runtime` / `Runtime::builder` / `RuntimeBuilder` (was `ProgramRuntime::new`),
  `ProgramInstance` (was `ProgramDef`; it now owns the stepper from `build`), `RuntimeMessage`
  (was `Message`). `RuntimeEffect`, `RuntimeEvent`, `RuntimeConfig`, `RuntimeLimits`,
  `EffectTable`, `Route`, `RunOutcome` keep their names.
- Ids: one v5 namespace per kind (`ProgramId::from_name`, `RouteId::of`, `TimerId::of`,
  `RunId::of_topology`); the experiment's `ProgramId::named` was the OID namespace shared by
  every kind.
- `RuntimeEvent` is `{ seq, tick, kind }`; every step record carries the program id, the input's
  source and its disposition.
- Admission policies: `Consume` (the experiment's only behaviour), `Retry { backlog,
  max_attempts }` and `Backpressure`, per runtime or per program, with waiter lists that wake held
  inputs when the place they wait for is freed.
- Timers whose program's mailbox is full wait for a place instead of being refused.
- New bounds: `routes`, `retry_backlog`, `trace_events` (oldest dropped, counted),
  `child_cancel_ticks` (was a constant).
- Runtime observers (`RuntimeObserver`, `EventCollector`, and `BowlRuntimeObserver` under
  `bowl`); `RunOutcome::diff` reports the first difference between two runs.
- Removed `RuntimeEffect::WallTimer` and the wall-clock example: real time is outside replay.

Proof: `cargo test` (6 unit, 12 admission, 2 observer, 8 policy, 5 replay, 7 scenario tests, 1
doctest, 4 compile-fail doctests), `cargo +1.88 test --all-features` (plus 2 bowl tests),
`cargo run --release --example contention` (90 configurations, 10 runs each, all equal).
