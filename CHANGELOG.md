# Changelog

Dates are when crates.io got the version, in UTC.

## 0.2.0 (not published)

Not published. The code is commit [5037941](https://github.com/jameshuntt/big-bad-bool-runtime/commit/5037941c40325db55e7104b66da8d1d0027d4256)
on the branch `scope-capacity-waiters`, from `main` at a6b3933; this entry and the docs are the commit
after it. Line links are at 5037941; they open once the branch is pushed. It needs scope 0.4.0, which
is not on crates.io yet: `Cargo.toml` says `scope = "0.4.0"` and, until scope 0.4.0 is published, a
`[patch.crates-io]` to `../scope` (branch `capacity-waiters`) marked "remove at release". With that
section the package cannot be verified against crates.io; it goes before publishing.

Minor version: scope's types are in this crate's API (`StepRecord::wake`, `SchedEvent`,
`RuntimeConfig::with_scheduler`), and scope goes from 0.3 to 0.4; for a 0.x crate that is a minor
bump. No item of this crate's own API changed.

Why: in 0.1.0 a freed mailbox place woke every program waiting for it, and all but one were refused
again. With 64 casters and one sink of 16 places (`policy/fan_in_64x16`) that was 11.41 (`Retry`) and
14.42 (`Backpressure`) refused attempts per committed step. scope 0.4.0 holds a freed place for the
oldest waiter and wakes only it.

### Changed

- Mailbox waits go through Scope. Before: [src/run.rs L132-147 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L132-L147) (`Shared::mailbox_waiters`),
  [src/run.rs L158-182 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L158-L182) (`Shared::wait` and `release`, a wake of every waiter with
  `WakeReason::Manual`), [src/run.rs L851-880 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L851-L880) (the program's own `free` read before and after
  each poll, `release` when it grew). After: [src/run.rs L462-493 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L462-L493), `wait_for`: a refusal
  `MailboxFull` for one place registers the task in the receiver's mailbox
  (`BoundedSender::wake_when_free`) and keeps the `CapacityWait`; [src/run.rs L495-512 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L495-L512),
  `settle_waits`: after each poll the task gives up every registration its refusals no longer name;
  [src/run.rs L528-560 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L528-L560), `end` gives up all of them. A given-up registration hands its held place
  to the next waiter. [src/run.rs L120-127 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L120-L127): admission counts the place Scope holds for the task
  (`free_for`); [src/run.rs L223-275 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L223-L275): every delivery goes through `try_send_from`.
- Two waits stay on a runtime list, because Scope cannot see the place they need:
  [src/run.rs L135-152 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L135-L152) (`Shared::place_waiters`), [src/run.rs L172-221 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L172-L221) (`release`,
  `wait_for_one_place`, `gave_up`). A sender to a program whose `Backpressure` input holds a place
  (Scope sees that place free and answers `FreeNow`), and a step that needs more than one place in
  one mailbox (it gives up a held place first, so two such senders never keep each other waiting).
  The receiving program wakes that list, with `WakeReason::Capacity` and a `Released` event, when it
  took a message it does not hold, committed or dropped its held input, or ended; a given-up
  registration wakes it too. The before/after compare is gone ([src/run.rs L970-998 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L970-L998)).
- The mailbox has exactly `RuntimeLimits::mailbox` places. Before: [src/run.rs L1053-1053 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L1053-L1053)
  (one more, kept for the shutdown word) and [src/run.rs L1131-1134 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L1131-L1134) (the word sent at shutdown).
  A place no sender may fill would look free to Scope and every full mailbox would answer `FreeNow`.
  After: [src/run.rs L1174-1174 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L1174-L1174) and [src/run.rs L1255-1260 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L1255-L1260): each program is woken at
  shutdown through `Scope::waker_for(.., WakeReason::Shutdown)`; the shutdown flag is read first on
  every poll, as before. The `Envelope` type ([src/run.rs L61-65 @ a6b3933](https://github.com/jameshuntt/big-bad-bool-runtime/blob/a6b393305c3066c775ccbc3ebb55c48c2880a4e2/src/run.rs#L61-L65)) is gone with the word.
- Timers: [src/run.rs L1003-1014 @ 5037941](https://github.com/jameshuntt/big-bad-bool-runtime/blob/5037941c40325db55e7104b66da8d1d0027d4256/src/run.rs#L1003-L1014), a timer whose parent's mailbox is full registers the same way
  and keeps its `CapacityWait` until it delivers.
- Trace: a mailbox place a waiter registered for is Scope's wake (`SchedEvent::Wake(.., Capacity)`,
  and the woken step's `wake` is `Some(Capacity)`), with no `Released` event; `Released` now marks the
  runtime's own lists only. The shutdown wake is `WakeReason::Shutdown` instead of `Inbox`.

### Proof

- `cargo test`: 48 without features, 50 with `bowl` (Rust 1.85 without features: 48). One assertion
  changed: `retry_backlogs_the_refused_input_and_the_same_input_commits_once_a_place_is_freed` looked
  for a `Released` event of the sink's mailbox; it now checks the second attempt's `wake` is
  `Capacity`, that the scheduler trace has exactly one capacity wake of the source, and that no
  `Released` names the sink's mailbox. New `tests/contention.rs`: 64 x 16 fan-in commits every input
  with at most one capacity wake per commit and no runtime list (1.41 and 0.49 refused per committed
  step); a 12-caster fan-in is the same run twice under every ordering and poll budget; a relay
  holding inputs and sending twice per step commits everything, through the runtime list.
- `cargo bench --bench runtime` (docs/MEASUREMENTS.md, both versions run on 2026-11-08):
  `policy/fan_in_64x16` `Retry` 47.4 ms to 9.93 ms, 23,363 to 2,881 refusals; `Backpressure` 60.9 ms
  to 6.43 ms, 29,526 to 1,008 refusals.
- The Custodian record (attempts_runtime_wake_validation): the Stage A run on the experiment's scope
  fork, then this runtime on scope 0.4.0 against 0.1.0 on scope 0.3.2, with every gate (results in
  that folder's `results/`).

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
