# Missing upstream APIs

What upstream did not have, what this crate does instead, and the smallest addition that would
remove the substitute. Nothing below blocked the implementation; each item costs something
named here.

## 1. scope: a wake when a bounded mailbox has room again

- **Status: added in scope 0.4.0, used since 0.2.0.**
- **Needed by:** `Retry` and `Backpressure`. A held input refused `MailboxFull` must run again
  when the receiver frees a place.
- **What 0.1.0 did (scope 0.3.2):** waiter lists in the runtime's Scope environment
  (`Shared::mailbox_waiters`), a compare of the program's own mailbox `free` before and after each
  poll, and a wake of every waiter (`WakeReason::Manual`) with a `Released` event when it grew. Under
  64 senders and one receiver (`policy/fan_in_64x16`) that was 11.4 (`Retry`) and 14.4
  (`Backpressure`) refused attempts per committed step.
- **What scope 0.4.0 has:** `BoundedSender::wake_when_free(task)` registers the task in the
  mailbox, in arrival order; a receive holds each freed place for the oldest waiter and wakes it
  (`WakeReason::Capacity`); `try_send_from(msg, task)` fills the held place; `free_for(task)`
  counts it for admission; the `CapacityWait` the registration returns gives it up when dropped.
- **Used here:** a program (or timer) refused `MailboxFull` for one place registers and keeps the
  `CapacityWait`; admission counts `free_for`; deliveries go through `try_send_from`; a poll gives
  up every registration its refusals no longer name, and the task's end gives up all of them.
  `mailbox_waiters` and the before/after compare are gone. With 64 senders and one receiver:
  1.41 (`Retry`) and 0.49 (`Backpressure`) refused attempts per committed step
  (docs/MEASUREMENTS.md).
- **What is left (cost):** two waits Scope cannot serve stay on a runtime list per mailbox
  (`Shared::place_waiters`), woken when the place may have come free, with a `Released` event: a
  sender to a program whose `Backpressure` input holds a place (item 2), and a step that needs
  more than one place in one mailbox (a held place is one place; the step gives it up and waits
  for unheld places, so two such senders never keep each other waiting, but it can wait long under
  single-place contention).
- **The shutdown word's place is gone too:** Scope 0.4 sees a place no sender may fill as free, so
  the kept place of item 4 would make every full mailbox look open. The mailbox has exactly
  `RuntimeLimits::mailbox` places and the run wakes each program at shutdown through
  `Scope::waker_for(.., WakeReason::Shutdown)`.

## 2. scope: look at the head of a bounded mailbox without taking it

- **Status: still missing.** Since 0.2.0 it is also why a runtime waiter list is left (item 1):
  Scope sees the place a held input takes as free, so a sender to that program cannot wait
  through Scope and waits on the runtime's list instead.
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

- **Status: not needed since 0.2.0.** 0.1.0 gave every mailbox `mailbox + 1` places and its own
  `free` subtracted the one kept for the shutdown word. That place is gone (item 1): the shutdown
  wake goes through `Scope::waker_for(.., WakeReason::Shutdown)`, which scope 0.3.2 already had, and
  the shutdown flag is read first on every poll.
- **Minimal addition (scope), if a control message is ever needed again:** a `try_send_reserved`
  (or a one-message control lane) on `BoundedSender`.

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
