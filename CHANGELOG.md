# Changelog

Dates are when crates.io got the version, in UTC.

## 0.1.0 (2026-11-07)

First release. A deterministic runtime for many verified big-bad-bool programs on one Scope,
on crates.io dependencies only: scope 0.3.2, big-bad-bool 0.1.2 with `program`
(big-bad-bool-program 0.1.1), uuid_macros 0.1.0, and ob-serve-a-bowl 0.2.3 behind `bowl`.
Line links are at [615a8fa](https://github.com/jameshuntt/big-bad-bool-runtime/commit/615a8fa49af70e2891d9e87270f4e4ba691655ec),
the commit before this release's.

Why: a big-bad-bool program decides what a step means, but nothing ran many programs together
with bounds a host can trust. Here the program keeps semantic authority, the runtime decides
routing, admission and policy, and Scope decides scheduling, lifetime and logical time.

### The runtime

- [src/run.rs L533-570](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/run.rs#L533-L570): one step is `Stepper::prepare`, admission
  of every runtime effect of the preparation at once, then `Stepper::commit` of exactly that
  preparation and only then the actions. A refused preparation is dropped: nothing is committed,
  nothing is performed, and the trace gets one `RuntimeEvent` with every reason and
  `pre == post`.
- [src/run.rs L255-362](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/run.rs#L255-L362): `admit` checks effects per step, routes,
  deliveries per step, each receiver's free mailbox places (one kept for the shutdown word),
  free timer and child places, child keys and the step budget, and collects every refusal.
- [src/config.rs L99-123](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/config.rs#L99-L123): `AdmissionPolicy` `Consume`,
  `Retry { backlog, max_attempts }` and `Backpressure`, per runtime or per program. A final
  refusal is consumed under every policy.
- [src/run.rs L143-182](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/run.rs#L143-L182): waiter lists for mailbox, timer and child
  places. The task that frees a place wakes every waiter, in the order they began waiting, and
  records `Released`. Scope 0.3.2 has no wake for "a bounded mailbox has room again"; this is
  the substitute, and under heavy fan-in it costs extra refused attempts (README, Limitations;
  docs/MISSING_APIS.md).
- [src/ids.rs L20-29](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/ids.rs#L20-L29): `ProgramId`, `RouteId`, `RunId`, `TimerId`
  are uuid_macros v5 newtypes, one namespace per kind; inside a run, programs are dense
  `ProgramSlot`s.
- Bounds in `RuntimeConfig::limits`, each with one deterministic result; `RunOutcome::same_run`
  and `RunOutcome::diff` compare runs; `RuntimeObserver` and `EventCollector`, and
  `BowlRuntimeObserver` under `bowl` (Rust 1.88), observe only.

### Public API

- [src/lib.rs L124-159](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/src/lib.rs#L124-L159): every module is private; the public API is
  the root re-exports. The test fixtures are not part of it: they are
  [tests/common/fixtures.rs](https://github.com/jameshuntt/big-bad-bool-runtime/blob/615a8fa49af70e2891d9e87270f4e4ba691655ec/tests/common/fixtures.rs), shared by the tests, examples,
  benches and doctests.

Proof: `cargo test` (6 unit, 12 admission, 2 observer, 8 policy, 5 replay, 7 scenario tests,
1 doctest, 4 compile-fail doctests), `cargo test --all-features` and
`cargo +1.88 test --all-features` (plus 2 bowl tests), `cargo +1.85 test`,
`cargo +nightly test --doc` (the compile-fail error codes checked),
`cargo run --release --example contention` (90 configurations, 10 runs each, all equal).
