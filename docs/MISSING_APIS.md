# Missing upstream APIs

What upstream did not have, what this crate does instead, and the smallest addition that would
remove the substitute. Nothing below blocked the implementation; each item costs something
named here.

## 1. scope: a wake when a bounded mailbox has room again

- **Needed by:** `Retry` and `Backpressure`. A held input refused `MailboxFull` must run again
  when the receiver frees a place.
- **What scope 0.3.2 has:** `BoundedMailbox::try_recv` frees a place and wakes nobody;
  `BoundedSender::free` reports places; `WakeReason` has no capacity reason.
- **Substitute here:** waiter lists in the runtime's Scope environment (`Shared::mailbox_waiters`,
  `timer_waiters`, `child_waiters`), filled from the refusal reasons. A program task compares its
  own mailbox's `free` before and after each poll (two extra locks per poll); when it grew, every
  waiter is woken with `Cx::wake_with(task, WakeReason::Manual)` in registration order and a
  `RuntimeEventKind::Released` event is recorded. Deterministic, because every receive happens
  inside the runtime's own polls.
- **Cost:** every waiter wakes for one freed place. Under 64 senders and one receiver
  (`policy/fan_in_64x16`) that is 11.4 (`Retry`) and 14.4 (`Backpressure`) refused attempts per
  commit (docs/MEASUREMENTS.md). Senders outside the runtime cannot wait this way.
- **Minimal addition (scope):** `BoundedSender::wake_when_free(&self, waiter: TaskId)`, a FIFO of
  waiters inside the bounded mailbox's shared state, and `WakeReason::Capacity`; `try_recv` and
  `drain` wake as many waiters as places they freed, `close` (and the read end's drop) wakes all.

## 2. scope: look at the head of a bounded mailbox without taking it

- **Needed by:** `Backpressure`, whose refused input should stay in the mailbox, still taking its
  place.
- **What scope 0.3.2 has:** `try_recv`, `drain`, `len`, `is_empty`; no peek.
- **Substitute here:** the input is taken and held in the program task, and the runtime counts it
  as one occupied place of that mailbox (`Directory::held`, subtracted in `Directory::free`), so
  senders see the same bound. The count lives beside the mailbox, not in it; a sender outside the
  runtime using `BoundedSender::free` directly would see one place more.
- **Minimal addition (scope):** `BoundedMailbox::peek_with<R>(&self, f: impl FnOnce(&M) -> R) ->
  Option<R>` (the lock is internal, so a closure, not a reference).

## 3. scope: the id of a task spawned from inside a poll

- **What scope 0.3.2 has:** `Cx::spawn_later_named` returns `()`.
- **Substitute here (kept from the registry experiment):** a child task records its own `TaskId`
  on its first poll; a `CancelChild` committed before then sets a flag the child reads then
  (`ActionRecord::ChildCancelRequested { task: None, .. }`).
- **Minimal addition (scope):** reserve the slot at `spawn_later_*` and return its `TaskId`.

## 4. scope: a reserved place for control messages

- **Substitute here (kept):** every mailbox has `mailbox + 1` places and the runtime's own `free`
  subtracts the one kept for the shutdown word.
- **Minimal addition (scope):** a `try_send_reserved` (or a one-message control lane) on
  `BoundedSender`.

## 5. uuid_macros: per-kind namespaces, ordering, and no forced v4

- `typed_uuid!`'s `named(label)` is v5 under the OID namespace for every kind, so
  `ProgramId::named("x")` and `RouteId::named("x")` are the same UUID. This crate uses
  `named_v5` with one namespace per kind (`ProgramId::from_name`, `RouteId::of`, `TimerId::of`,
  `RunId::of_topology`); the generated `named` and `new_v4` exist on the types and are never
  called here.
- `typed_uuid!` types are not `PartialOrd`/`Ord`, so the builder keys its index by `id.0`.
- uuid_macros 0.1.0 always enables `uuid/v4`, which pulls `getrandom` into every dependent
  (`cargo tree` shows getrandom 0.4.3 under uuid), although no random id is made.
- **Minimal addition (uuid_macros):** `named_kind(name)` = `named_v5(KIND, name)`, an `Ord` derive
  (as `typed_uuid_2!` has), and a `v4` feature for `new_v4`.

## 6. big-bad-bool-program: a state digest

- **Substitute here (kept):** `state_digest` hashes the spec digest and every dimension's word and
  stack through `ProjectionState`'s public accessors.
- **Minimal addition (big-bad-bool-program):** `ProjectionState::digest(&self, program:
  &VerifiedProgram) -> StableDigest`, so every host digests states the same way.

## Not missing

- `big_bad_bool::program` (the facade's `program` feature) carries everything this crate uses,
  macros included (`dimension!` expands through `$crate`); there is no direct dependency on
  big-bad-bool-program.
- A `PreparedStep` cannot be kept across another commit (`CommitRefusal::Stale`), so a held input
  is re-prepared at each attempt from the current state. That is the program crate's rule and the
  right one: an attempt is always against the state it would change.
