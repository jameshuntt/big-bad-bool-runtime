# Measurements

`cargo bench --bench runtime` (benches/runtime.rs), 2026-11-07, 09:31:55 to 09:33:21 EST (86 s),
one run. Criterion 0.5.1, 10 samples, 0.5 s warm-up, 2 s measurement per benchmark, as the
registry experiment and v2. Numbers are Criterion's [low estimate high]. Nothing was optimized.

Machine: Apple M4 Pro, 14 cores, 48 GiB, macOS 15.7.5, **on battery** (72% to 71%,
discharging). rustc 1.97.1 (8bab26f4f 2026-07-14), `bench` profile. Dependencies from crates.io:
scope 0.3.2, big-bad-bool 0.1.2 + big-bad-bool-program 0.1.1, uuid_macros 0.1.0.

The v2 column is copied from `attempts_scope_programs_registry/docs/MEASUREMENTS.md`: v2's own
run on path dependencies (big-bad-bool-program `prepared-step` bddc736, scope
`logical-runtime-primitives` d5dc5a1), same machine and settings, on the charger. The registry
experiment never measured itself on the crates.io versions, and it was not re-run here (no cargo
command was run inside that repository). The two columns differ in power state and in dependency
builds as well as in code, so a difference between them is not attributed to any one cause.

Every runtime benchmark builds the runtime, runs it, shuts it down and collects the final states
inside the timed loop. Program ids are made once outside the loops.

| Benchmark | What it is | this crate | per item | v2 | v2 per item |
|---|---|---|---|---|---|
| steps/bare_stepper_push | 10,000 `Stepper::push` on `volley`, no runtime | [9.9517 10.037 10.096] ms | 1.004 µs/step | [10.453 10.510 10.566] ms | 1.051 µs/step |
| steps/prepare_commit | 10,000 `prepare`, read effects, `commit` | [10.016 10.103 10.178] ms | 1.010 µs/step | [10.457 10.502 10.526] ms | 1.050 µs/step |
| steps/runtime_prepare_admit_commit_unrouted | one sink, 10,000 injected hits: prepare, admit, commit | [23.656 23.793 23.925] ms | 2.38 µs/step | [24.686 24.804 24.871] ms | 2.48 µs/step |
| steps/runtime_volley_routed | two volleys, 10,000 steps, one routed message each | [15.743 15.864 15.967] ms | 1.586 µs/step | [16.703 16.783 16.862] ms | 1.678 µs/step |
| steps/runtime_volley_routed_full_trace | the same, `TraceLevel::Full` | [35.563 35.708 35.995] ms | 3.57 µs/step | [35.730 35.925 36.152] ms | 3.59 µs/step |
| admission/refusal_prepare_admit_refuse | one volley, 10,000 injected hits, each prepared and refused (`NoRoute`, final), nothing commits | [13.596 13.651 13.703] ms | 1.365 µs/refusal | (none) | |
| policy/fan_in_64x16/consume | 64 casters x 16 starts into one sink with 16 places: 62 commits, 993 refusals | [2.1394 2.1470 2.1561] ms | 2.03 µs/attempt | (none) | |
| policy/fan_in_64x16/retry | the same under `Retry { backlog: 16 }`: 2048 commits, 23,363 refusals | [46.621 46.760 46.929] ms | 1.84 µs/attempt, 22.8 µs/commit | (none) | |
| policy/fan_in_64x16/backpressure | the same under `Backpressure`: 2048 commits, 29,526 refusals | [59.735 60.158 60.399] ms | 1.91 µs/attempt, 29.4 µs/commit | (none) | |
| timer/logical_metronome_1000_beats | one program, each step arms a 1-tick logical timer | [1.8332 1.8457 1.8615] ms | 1.85 µs/beat | [1.8621 1.8706 1.8814] ms | 1.87 µs/beat |
| scheduler/scope_mailbox_bounce | Scope alone, unbounded mailboxes, 10,000 messages | [492.64 495.23 498.36] µs | 49.5 ns/msg | [499.86 501.19 502.17] µs | 50.1 ns/msg |
| scheduler/scope_bounded_bounce | the same over bounded mailboxes (capacity 4) and `try_send` | [495.21 498.81 502.17] µs | 49.9 ns/msg | [505.43 507.72 510.26] µs | 50.8 ns/msg |
| instances/independent_sinks_4_hits/1 | 1 sink, 4 injected hits | [13.520 13.562 13.618] µs | 3.39 µs/step | [13.871 14.010 14.133] µs | 3.50 µs/step |
| instances/independent_sinks_4_hits/10 | 10 sinks | [123.49 123.91 124.41] µs | 3.10 µs/step | [129.62 130.55 131.53] µs | 3.26 µs/step |
| instances/independent_sinks_4_hits/100 | 100 sinks | [1.1661 1.1714 1.1769] ms | 2.93 µs/step | [1.2410 1.2528 1.2651] ms | 3.13 µs/step |
| instances/independent_sinks_4_hits/1000 | 1000 sinks | [11.802 11.871 11.952] ms | 2.97 µs/step | [12.665 12.728 12.797] ms | 3.18 µs/step |
| instances/rally_pairs_instances/2 | 1 rally pair (9 steps) | [28.435 28.572 28.700] µs | 3.17 µs/step | [30.646 30.840 31.126] µs | 3.43 µs/step |
| instances/rally_pairs_instances/10 | 5 pairs | [139.69 140.08 140.82] µs | 3.11 µs/step | [147.23 148.53 149.83] µs | 3.30 µs/step |
| instances/rally_pairs_instances/100 | 50 pairs | [1.3269 1.3315 1.3410] ms | 2.96 µs/step | [1.4075 1.4354 1.4876] ms | 3.19 µs/step |
| instances/rally_pairs_instances/1000 | 500 pairs | [13.456 13.503 13.565] ms | 3.00 µs/step | [14.698 14.823 14.977] ms | 3.29 µs/step |
| fan_out/one_commit_to_k_sinks/1 | one `announce` to 1 sink | [9.8602 9.9263 9.9734] µs | | [10.377 10.397 10.436] µs | |
| fan_out/one_commit_to_k_sinks/8 | to 8 sinks | [42.535 42.828 43.362] µs | | [45.950 46.326 46.928] µs | |
| fan_out/one_commit_to_k_sinks/64 | to 64 sinks | [291.70 293.23 295.67] µs | 4.47 µs per added sink (8 to 64) | [320.21 322.24 324.28] µs | 4.93 µs |
| trace/rally_100_pairs/off | 100 pairs, 900 steps | [2.6391 2.6440 2.6506] ms | 2.94 µs/step | [2.8057 2.8156 2.8272] ms | 3.13 µs/step |
| trace/rally_100_pairs/full | the same, full trace | [4.4422 4.4943 4.5569] ms | +2.06 µs/step | [4.7847 4.7943 4.8093] ms | +2.20 µs/step |
| idle/volley_2000_steps_with_idle/0 | 2,000 volley steps | [3.1761 3.1981 3.2241] ms | | [3.1961 3.2120 3.2295] ms | |
| idle/volley_2000_steps_with_idle/100 | plus 100 idle programs | [3.3525 3.3665 3.3739] ms | +1.68 µs per idle program | [3.4461 3.4550 3.4723] ms | +2.43 µs |
| idle/volley_2000_steps_with_idle/1000 | plus 1000 | [4.7657 4.8112 4.8451] ms | +1.61 µs per idle program | [5.0344 5.0496 5.0691] ms | +1.84 µs |

What the numbers say, and only that:

- On the shapes both measured, this crate's numbers are 0% to 9% below v2's. This crate does at
  least as much per step as v2 (the same prepare, table lookup, admission and bounded send, plus
  the held-input check, two `free` reads of the program's own mailbox per poll, and the program
  id in every step record). The runs differ in power state and dependency builds; the share of
  each cause was not measured.
- A refused attempt (prepare, table lookup, admission, record of nothing) costs 1.37 µs against
  2.38 µs for a committed unrouted step on the runtime.
- From 1 to 1000 instances the cost per step stays between 2.9 and 3.4 µs: linear.
- Under contention (64 senders, one receiver taking one message per poll), `Retry` and
  `Backpressure` commit every input (2048 against 62 under `Consume`), and spend 11.4 and 14.4
  refused attempts per commit. The cause is the wake rule: a freed place wakes every task waiting
  for it, all of them attempt again, one gets the place. A wake of one waiter per freed place
  would need Scope to say how many places a receive freed and to whom (docs/MISSING_APIS.md,
  item 1). Per attempt, the policies cost what a plain attempt costs (1.84 to 2.03 µs).
- The full trace costs 2.06 µs per step on top.
