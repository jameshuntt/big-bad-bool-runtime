# Measurements

## 0.2.0: waiting through scope 0.4.0's capacity waiters (2026-11-08)

`cargo bench --bench runtime`, the same benches and settings as below, run twice in a row on
2026-11-08: 0.1.0 (commit a6b3933, built in attempts_runtime_wake_validation/bench_before with its
own Cargo.lock: scope 0.3.2 from crates.io) at 08:34:06 to 08:35:30 EST, then this branch (commit
5037941: scope 0.4.0 from the `capacity-waiters` checkout through the `[patch.crates-io]`) at
08:40:53 to 08:42:14 EST. Apple M4 Pro, macOS 15.7.5, **on battery** (87% to 83%, discharging),
rustc 1.97.1. Numbers are Criterion's [low estimate high]. Nothing was optimized for the bench.

| Benchmark | 0.1.0 | 0.2.0 | change |
|---|---|---|---|
| steps/bare_stepper_push | [10.248 ms 10.335 ms 10.416 ms] | [10.366 ms 11.238 ms 12.570 ms] | +8.7% |
| steps/prepare_commit | [10.152 ms 10.213 ms 10.268 ms] | [10.293 ms 10.349 ms 10.403 ms] | +1.3% |
| steps/runtime_prepare_admit_commit_unrouted | [24.292 ms 24.386 ms 24.444 ms] | [24.315 ms 24.423 ms 24.646 ms] | +0.2% |
| steps/runtime_volley_routed | [16.066 ms 16.130 ms 16.199 ms] | [16.067 ms 16.165 ms 16.286 ms] | +0.2% |
| steps/runtime_volley_routed_full_trace | [36.689 ms 36.871 ms 36.972 ms] | [35.917 ms 37.279 ms 40.190 ms] | +1.1% |
| admission/refusal_prepare_admit_refuse | [13.861 ms 13.923 ms 13.961 ms] | [13.878 ms 13.962 ms 14.056 ms] | +0.3% |
| policy/fan_in_64x16/consume | [2.1668 ms 2.1799 ms 2.1881 ms] | [2.1467 ms 2.1569 ms 2.1692 ms] | -1.1% |
| policy/fan_in_64x16/retry | [47.168 ms 47.399 ms 47.555 ms] | [9.5764 ms 9.9317 ms 10.437 ms] | -79.0% |
| policy/fan_in_64x16/backpressure | [60.595 ms 60.940 ms 61.398 ms] | [6.3959 ms 6.4283 ms 6.5038 ms] | -89.5% |
| timer/logical_metronome_1000_beats | [1.8715 ms 1.8829 ms 1.8959 ms] | [1.8159 ms 1.8339 ms 1.8527 ms] | -2.6% |
| scheduler/scope_mailbox_bounce | [486.69 µs 490.18 µs 493.16 µs] | [482.76 µs 483.35 µs 484.09 µs] | -1.4% |
| scheduler/scope_bounded_bounce | [492.25 µs 495.43 µs 500.09 µs] | [489.40 µs 492.71 µs 495.20 µs] | -0.5% |
| instances/independent_sinks_4_hits/1 | [13.652 µs 13.784 µs 13.893 µs] | [13.548 µs 13.629 µs 13.803 µs] | -1.1% |
| instances/independent_sinks_4_hits/10 | [126.07 µs 127.63 µs 128.73 µs] | [125.09 µs 125.77 µs 126.42 µs] | -1.5% |
| instances/independent_sinks_4_hits/100 | [1.2122 ms 1.3144 ms 1.3715 ms] | [1.1752 ms 1.1797 ms 1.1878 ms] | -10.2% |
| instances/independent_sinks_4_hits/1000 | [15.650 ms 16.281 ms 17.063 ms] | [11.709 ms 11.733 ms 11.754 ms] | -27.9% |
| instances/rally_pairs_instances/2 | [38.317 µs 40.850 µs 42.875 µs] | [29.024 µs 29.247 µs 29.406 µs] | -28.4% |
| instances/rally_pairs_instances/10 | [176.05 µs 188.33 µs 196.57 µs] | [140.19 µs 140.54 µs 140.79 µs] | -25.4% |
| instances/rally_pairs_instances/100 | [1.6649 ms 1.7489 ms 1.8469 ms] | [1.3335 ms 1.3399 ms 1.3460 ms] | -23.4% |
| instances/rally_pairs_instances/1000 | [19.844 ms 20.333 ms 20.726 ms] | [13.386 ms 13.610 ms 13.786 ms] | -33.1% |
| fan_out/one_commit_to_k_sinks/1 | [11.997 µs 12.446 µs 12.876 µs] | [9.9880 µs 10.075 µs 10.204 µs] | -19.1% |
| fan_out/one_commit_to_k_sinks/8 | [56.994 µs 59.801 µs 61.845 µs] | [43.153 µs 43.241 µs 43.370 µs] | -27.7% |
| fan_out/one_commit_to_k_sinks/64 | [323.24 µs 342.73 µs 385.20 µs] | [305.60 µs 329.70 µs 359.34 µs] | -3.8% |
| trace/rally_100_pairs/off | [2.7592 ms 2.7707 ms 2.7828 ms] | [2.7940 ms 2.8150 ms 2.8588 ms] | +1.6% |
| trace/rally_100_pairs/full | [4.8638 ms 5.2263 ms 5.4651 ms] | [4.5421 ms 4.5857 ms 4.6567 ms] | -12.3% |
| idle/volley_2000_steps_with_idle/0 | [3.2985 ms 3.3424 ms 3.3765 ms] | [3.1604 ms 3.1907 ms 3.2236 ms] | -4.5% |
| idle/volley_2000_steps_with_idle/100 | [3.4661 ms 3.4904 ms 3.5259 ms] | [3.3844 ms 3.4162 ms 3.4574 ms] | -2.1% |
| idle/volley_2000_steps_with_idle/1000 | [4.7865 ms 4.9466 ms 5.0616 ms] | [4.7728 ms 4.8427 ms 4.8930 ms] | -2.1% |

What the numbers say, and only that:

- `policy/fan_in_64x16` (64 casters x 16 starts, one sink with 16 places): under `Retry` 2,881
  refused attempts for the 2,048 committed steps, 1.41 per step, against 23,363 (11.41 per step) in
  0.1.0; under `Backpressure` 1,008 (0.49 per step) against 29,526 (14.42). The time follows: 47.4 ms
  to 9.93 ms (`Retry`), 60.9 ms to 6.43 ms (`Backpressure`). `Consume` refuses the same 993. A freed
  place now wakes one waiting caster and is held for it, where 0.1.0 woke every waiting caster.
- The other shapes have nobody waiting for a mailbox place. The ones that run no runtime code at all
  (`steps/bare_stepper_push`, `steps/prepare_commit`) moved +8.7% and +1.3% between the two runs,
  and the 0.1.0 run's `instances/*` and `fan_out/*` estimates have wide ranges, so differences of
  that size here are the two runs, not the change. `scheduler/scope_bounded_bounce` (Scope's bounded
  mailbox alone, no waiter) is -0.5%: the no-waiter paths of scope 0.4.0's `try_recv` and
  `try_send` cost what 0.3.2's did (at scope d6bf9d2 this bench was 5% slower; 7100203 fixed it).
- The Custodian record in attempts_runtime_wake_validation runs the same runtime on both scopes over
  a grid of senders, places, policies, orderings and workloads, with every gate; its uncontended
  nodes put the change within 2% per commit.

## 0.1.0 (2026-11-07)

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
