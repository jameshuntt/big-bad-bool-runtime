# Sharding: not implemented

Question: could several workers, each with its own Scope and a partition of the programs,
exchange messages without changing program semantics, deterministically?

## Shape

- Each worker runs one `Runtime` on its own Scope over its own programs (its own slots, mailboxes,
  timers, children, waiters and trace).
- Each pair of workers has a bounded outbox. A step whose `Send` reaches a program on another
  worker is admitted against a **credit**: the number of places the receiving worker granted the
  sender in that receiver's mailbox for this epoch. Within credit, the delivery goes to the
  outbox; past it, admission refuses the step like a local `MailboxFull`, before the commit, and
  the step's policy applies.
- Epochs are the exchange points the runtime already has (`inject_in_epoch`): every worker runs
  its epoch until idle, outboxes are swapped, incoming messages are injected sorted by
  `(sending worker, message sequence)`, unused credit returns, new credit is granted, the next
  epoch runs. The run is then a function of each worker's wake sequence, policy and tick rule,
  plus the fixed exchange order.
- The program model does not change: a program sees only the order of its own messages, and
  `prepare` / `commit` are local to its stepper.

## What must change in this crate's types

- `MessageId` is one counter per run; it becomes `(worker, sequence)`. `Origin` gains the worker.
- A route target is a slot of one run (`Shared::routes`); it becomes `Local(slot)` or
  `Remote(worker, ProgramId)`. `ProgramId` is v5 by name, so every worker computes the same id.
- Admission gains a credit check per remote receiver (`RuntimeRefusal::NoCredit { worker, to }`,
  not final: credit returns at the next exchange).
- Wakes do not cross workers. A held input refused for credit waits for the next exchange instead
  of a `Released` wake: `Resource::Credit(worker)`, released by the exchange.
- `Backpressure` across workers: a held input keeps its local place as now; the remote side sees
  pressure as credit not granted, one epoch later.
- The step budget and the timer and child bounds are per worker (or divided); ticks are per
  worker's Scope, and an epoch boundary is the only cross-worker time.
- `RunOutcome` holds one trace per worker plus the exchange record of every epoch;
  `same_run` compares all of them.
- Moving a program between workers needs a parked stepper (`Stepper::park` / `resume`); the
  program crate has none.

## Verdict

Feasible without touching the program model. The epoch loop, receiver-side bounds, durable ids,
admission before commit and the waiter mechanism all exist; credits, worker-qualified message
ids, remote route targets and the exchange are the work. Not started.
