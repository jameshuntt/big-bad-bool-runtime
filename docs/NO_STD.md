# no_std

This crate is `std` today, and cannot become `no_std` while scope needs `std`.

## What needs std upstream

- **scope 0.3.2:** `std::sync::{Arc, Mutex}` in every mailbox and the wake bus,
  `std::panic::catch_unwind` to isolate a panicking task, `std::time::{Instant, Duration}` for its
  wall-clock timers and cancel budgets, `std::task::Waker`, and `String` task names
  (`RunReport::stalled`). The runtime uses none of the wall-clock parts, but they are not
  features.
- **uuid_macros 0.1.0:** turns on `uuid/v4`, which brings `getrandom`.
- **big-bad-bool-program 0.1.1:** `Vec` in `Observation`, `ProjectionState`, `PreparedStep` and
  `StepResult`; refusal codes as `String` through liaise. `alloc` would do; it is not marked
  `no_std`.
- `stable_digest` 0.1.1 and big-bad-bool-core 0.1.1 are already `no_std`.

## What needs alloc in this crate, and what could be fixed-capacity

Every bound below is already a number in `RuntimeLimits`, so each structure has a known largest
size. ABUT 0.2.2's `HeaplessRing<FRAME_MAX, DEPTH>` (a `heapless::spsc::Queue` of fixed-size
frames, refusing with `RingError::Full`) is the reference shape for the rings; no dependency on
it.

| Structure | Today | Fixed-capacity form | Bound |
|---|---|---|---|
| `Retry` backlog | `VecDeque`, allocated once at `backlog` | ring of `RETRY_BACKLOG` entries | `retry_backlog` |
| retained trace | `VecDeque`, oldest dropped | ring of `TRACE_EVENTS` events, overwrite oldest | `trace_events` |
| waiter lists | `Vec<TaskId>` per mailbox, timers, children | array of `PROGRAMS + TIMERS` task ids per list | `programs`, `timers` |
| planned actions, effects of a step | `Vec` | array of `EFFECTS_PER_STEP` | `effects_per_step` |
| deliveries per target in `admit` | `BTreeMap` | array of `DELIVERIES_PER_STEP` | `deliveries_per_step` |
| children | `BTreeMap<(slot, key), _>` | array of `CHILDREN` slots | `children` |
| routes | `Vec<Vec<(Port, Vec<slot>)>>` | flat array of `ROUTES`, indexed per program | `routes`, `fanout` |
| `RuntimeMessage` | `Vec<(&'static str, u128)>` | array of `EVIDENCE` pairs | none yet: would need a bound |
| `EffectTable` / resolved table | `Vec` / `BTreeMap` | sorted array, binary search | none yet |
| `StepRecord` results, effects, actions, reasons | `Vec` | arrays of the bounds above | as above |
| topology text for the run id | `String` | a streaming v5 hash | none |
| `RunOutcome` | `Vec`s | caller-provided buffers | as above |

`Cell`, `RefCell` and `OnceCell` are `core`; `Rc` (the shared Scope tracer state) is `alloc`.

Making these fixed-capacity means const-generic bounds on `Runtime` instead of runtime numbers
in `RuntimeConfig`; that is an API change, not started.
