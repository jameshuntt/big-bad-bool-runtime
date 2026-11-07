// Small verified programs the tests, examples, benches and docs run. Test data, not part of
// the crate: tests reach it as `common::fixtures`, examples and benches with
// `#[path = "../tests/common/fixtures.rs"]`, doctests with `include!`.
//
// Every program reads one evidence dimension `signal`, keeps one state dimension, and most
// emit one effect dimension `out`. Each `*_table` function is a program's `EffectTable`:
// data, one rule per effect word.

use big_bad_bool::program::{
    Clause, ContainmentSpec, ContractSpec, FieldMatcher, Program, ProgramBuilder, ProgramSpec,
    RefusalRule, VerifiedProgram, VerifyLimits, clause, contract, dimension, set, verify,
};
use scope::TickDelta;

use big_bad_bool_runtime::{ChildKey, ChildSpec, EffectTable, Port, RuntimeEffect, RuntimeMessage};

/// The `#[bitpack]` word types of the fixtures. The derive generates their constants and
/// accessors, which carry no docs of their own.
#[allow(missing_docs)]
mod words {
    use big_bad_bool::bitpack;

    /// What one message says.
    #[bitpack(storage = u8)]
    pub struct Signal {
        /// The signal kind.
        #[bits(3)]
        #[labels(None = 0, Start = 1, Hit = 2, Alarm = 3, Tick = 4)]
        #[invalid(5..=7)]
        pub kind: u8,
    }

    /// A small counter.
    #[bitpack(storage = u8)]
    pub struct Count {
        /// The count.
        #[bits(3)]
        pub value: u8,
    }

    /// Where an alarm is.
    #[bitpack(storage = u8)]
    pub struct Phase {
        /// The phase.
        #[bits(2)]
        #[labels(Idle = 0, Waiting = 1, Done = 2)]
        #[invalid(3)]
        pub kind: u8,
    }

    /// What a program asks for.
    #[bitpack(storage = u8)]
    pub struct Out {
        /// The request.
        #[bits(3)]
        #[labels(
            None = 0,
            Hit = 1,
            Done = 2,
            Arm = 3,
            Ring = 4,
            StartChild = 5,
            StopChild = 6
        )]
        #[invalid(7)]
        pub kind: u8,
    }
}

pub use words::{Count, Out, Phase, Signal};

/// `signal.kind = Start`.
pub const START: u128 = Signal::LABEL_KIND_START.value;
/// `signal.kind = Hit`.
pub const HIT: u128 = Signal::LABEL_KIND_HIT.value;
/// `signal.kind = Alarm`.
pub const ALARM: u128 = Signal::LABEL_KIND_ALARM.value;
/// `signal.kind = Tick`.
pub const TICK: u128 = Signal::LABEL_KIND_TICK.value;

const OUT_HIT: u128 = Out::LABEL_KIND_HIT.value;
const OUT_DONE: u128 = Out::LABEL_KIND_DONE.value;
const OUT_ARM: u128 = Out::LABEL_KIND_ARM.value;
const OUT_RING: u128 = Out::LABEL_KIND_RING.value;
const OUT_START_CHILD: u128 = Out::LABEL_KIND_STARTCHILD.value;
const OUT_STOP_CHILD: u128 = Out::LABEL_KIND_STOPCHILD.value;
const IDLE: u128 = Phase::LABEL_KIND_IDLE.value;
const WAITING: u128 = Phase::LABEL_KIND_WAITING.value;
const DONE: u128 = Phase::LABEL_KIND_DONE.value;

/// External tag of a rally's `Done`.
pub const TAG_RALLY_DONE: u32 = 1;
/// External tag of an alarm's `Ring`.
pub const TAG_RING: u32 = 2;
/// How many scheduler rounds an alarm's logical timer waits.
pub const ALARM_TICKS: TickDelta = TickDelta(5);
/// How many ticks a supervisor's child would deliver if nobody cancelled it.
pub const TICKER_TICKS: u32 = 1_000;
/// Hits a rally commits before it reports `Done`.
pub const RALLY: u128 = 4;

const fn signal(kind: u128) -> Clause {
    clause("signal", FieldMatcher::eq("kind", kind))
}
const fn count(value: u128) -> Clause {
    clause("count", FieldMatcher::eq("value", value))
}
const fn phase(kind: u128) -> Clause {
    clause("phase", FieldMatcher::eq("kind", kind))
}

/// The message `signal.kind = kind`.
#[must_use]
pub fn signal_message(kind: u128) -> RuntimeMessage {
    RuntimeMessage::new().with("signal", kind)
}

fn containment(on_refusal: RefusalRule) -> ContainmentSpec {
    ContainmentSpec {
        max_interpretations: 1,
        max_candidates: 4,
        max_actualized: 4,
        max_stack_depth: 1,
        on_refusal,
    }
}

fn build(base: ProgramBuilder, contracts: &[ContractSpec], on_refusal: RefusalRule) -> ProgramSpec {
    contracts
        .iter()
        .fold(base.contain(containment(on_refusal)), |p, c| p.contract(*c))
        .build()
        .expect("every name resolves")
}

/// Verifies `spec` with the bounds every fixture uses.
///
/// # Panics
///
/// When the fixture does not verify.
#[must_use]
pub fn verified(spec: ProgramSpec) -> VerifiedProgram {
    verify(
        spec,
        VerifyLimits {
            max_states: 1_000,
            max_witness_tries: 1_024,
        },
    )
    .expect("the fixture verifies")
}

fn counter_base() -> ProgramBuilder {
    Program::new()
        .add(dimension!(Signal, "signal", Evidence))
        .add(dimension!(Count, "count", State))
        .add(dimension!(Out, "out", Effect))
}

macro_rules! counted {
    ($name:literal, $on:expr, $from:literal, $to:literal, $emit:expr) => {
        contract($name)
            .when(&[signal($on)])
            .require(&[count($from)])
            .transition(&[set("count", "value", $to)])
            .emit(&[set("out", "kind", $emit)])
    };
}

const RALLY_CONTRACTS: &[ContractSpec] = &[
    counted!("hit-0", HIT, 0, 1, OUT_HIT),
    counted!("hit-1", HIT, 1, 2, OUT_HIT),
    counted!("hit-2", HIT, 2, 3, OUT_HIT),
    counted!("hit-3", HIT, 3, 4, OUT_HIT),
    contract("rally-done")
        .when(&[signal(HIT)])
        .require(&[count(4)])
        .emit(&[set("out", "kind", OUT_DONE)]),
];

/// Counts `RALLY` hits, returning each one on port 0, then reports `Done` externally.
#[must_use]
pub fn rally() -> ProgramSpec {
    build(
        counter_base(),
        RALLY_CONTRACTS,
        RefusalRule::SkipObservation,
    )
}

fn send_hit() -> RuntimeEffect {
    RuntimeEffect::Send {
        port: Port(0),
        message: signal_message(HIT),
    }
}

/// A rally's or a volley's table: `Hit` goes out on port 0, `Done` goes to the application.
#[must_use]
pub fn rally_table() -> EffectTable {
    EffectTable::new().on("out", OUT_HIT, [send_hit()]).on(
        "out",
        OUT_DONE,
        [RuntimeEffect::External {
            tag: TAG_RALLY_DONE,
            word: OUT_DONE,
        }],
    )
}

/// A rally's table with 16 trace marks added to every `Hit`: 17 effects, one past
/// [`MAX_EFFECTS_PER_STEP`](big_bad_bool_runtime::MAX_EFFECTS_PER_STEP).
#[must_use]
pub fn flood_table() -> EffectTable {
    let hit = std::iter::once(send_hit()).chain((0..16).map(|tag| RuntimeEffect::Trace { tag }));
    EffectTable::new().on("out", OUT_HIT, hit)
}

/// Every `Hit` goes out on port 0 twice: two deliveries per route.
#[must_use]
pub fn twice_table() -> EffectTable {
    EffectTable::new().on("out", OUT_HIT, [send_hit(), send_hit()])
}

const VOLLEY_CONTRACTS: &[ContractSpec] = &[contract("return")
    .when(&[signal(HIT)])
    .emit(&[set("out", "kind", OUT_HIT)])];

/// Returns every hit on port 0, forever: only the runtime's step budget ends two volleys.
#[must_use]
pub fn volley() -> ProgramSpec {
    build(
        counter_base(),
        VOLLEY_CONTRACTS,
        RefusalRule::SkipObservation,
    )
}

const ALARM_CONTRACTS: &[ContractSpec] = &[
    contract("arm")
        .when(&[signal(START)])
        .require(&[phase(IDLE)])
        .transition(&[set("phase", "kind", WAITING)])
        .emit(&[set("out", "kind", OUT_ARM)]),
    contract("ring")
        .when(&[signal(ALARM)])
        .require(&[phase(WAITING)])
        .transition(&[set("phase", "kind", DONE)])
        .emit(&[set("out", "kind", OUT_RING)]),
    contract("rest")
        .when(&[signal(HIT)])
        .require(&[phase(DONE)]),
];

/// `Start` arms a timer; the timer's `Alarm` rings. The program holds no clock.
#[must_use]
pub fn alarm() -> ProgramSpec {
    let base = Program::new()
        .add(dimension!(Signal, "signal", Evidence))
        .add(dimension!(Phase, "phase", State))
        .add(dimension!(Out, "out", Effect));
    build(base, ALARM_CONTRACTS, RefusalRule::SkipObservation)
}

fn ring() -> RuntimeEffect {
    RuntimeEffect::External {
        tag: TAG_RING,
        word: OUT_RING,
    }
}

fn logical_alarm() -> RuntimeEffect {
    RuntimeEffect::Timer {
        after: ALARM_TICKS,
        message: signal_message(ALARM),
    }
}

/// An alarm's table: `Arm` asks Scope for a logical timer that delivers `Alarm` after
/// [`ALARM_TICKS`] rounds.
#[must_use]
pub fn alarm_table() -> EffectTable {
    EffectTable::new()
        .on("out", OUT_ARM, [logical_alarm()])
        .on("out", OUT_RING, [ring()])
}

/// An alarm whose `Arm` asks for two logical timers.
#[must_use]
pub fn two_timers_table() -> EffectTable {
    EffectTable::new()
        .on("out", OUT_ARM, [logical_alarm(), logical_alarm()])
        .on("out", OUT_RING, [ring()])
}

const METRONOME_CONTRACTS: &[ContractSpec] = &[
    contract("start")
        .when(&[signal(START)])
        .emit(&[set("out", "kind", OUT_ARM)]),
    contract("beat")
        .when(&[signal(ALARM)])
        .emit(&[set("out", "kind", OUT_ARM)]),
];

/// `Start`, and every `Alarm` after it, arms the next beat: one logical timer per step, forever.
/// Only the runtime's step budget ends it.
#[must_use]
pub fn metronome() -> ProgramSpec {
    build(
        counter_base(),
        METRONOME_CONTRACTS,
        RefusalRule::SkipObservation,
    )
}

/// A metronome's table: `Arm` is a logical timer of one round.
#[must_use]
pub fn metronome_table() -> EffectTable {
    EffectTable::new().on(
        "out",
        OUT_ARM,
        [RuntimeEffect::Timer {
            after: TickDelta(1),
            message: signal_message(ALARM),
        }],
    )
}

const SUPERVISOR_CONTRACTS: &[ContractSpec] = &[
    counted!("start", START, 0, 1, OUT_START_CHILD),
    contract("tick-1")
        .when(&[signal(TICK)])
        .require(&[count(1)])
        .transition(&[set("count", "value", 2)]),
    contract("tick-2")
        .when(&[signal(TICK)])
        .require(&[count(2)])
        .transition(&[set("count", "value", 3)]),
    counted!("tick-3", TICK, 3, 4, OUT_STOP_CHILD),
    contract("late").when(&[signal(TICK)]).require(&[count(4)]),
];

/// `Start` starts a ticking child; the third tick cancels it.
#[must_use]
pub fn supervisor() -> ProgramSpec {
    build(
        counter_base(),
        SUPERVISOR_CONTRACTS,
        RefusalRule::SkipObservation,
    )
}

/// A supervisor's table.
#[must_use]
pub fn supervisor_table() -> EffectTable {
    EffectTable::new()
        .on(
            "out",
            OUT_START_CHILD,
            [RuntimeEffect::StartChild {
                key: ChildKey(0),
                child: ChildSpec::Ticker {
                    ticks: TICKER_TICKS,
                    message: signal_message(TICK),
                },
            }],
        )
        .on(
            "out",
            OUT_STOP_CHILD,
            [RuntimeEffect::CancelChild { key: ChildKey(0) }],
        )
}

const BROADCASTER_CONTRACTS: &[ContractSpec] = &[
    counted!("announce", START, 0, 1, OUT_HIT),
    contract("again")
        .when(&[signal(START)])
        .require(&[count(1)])
        .emit(&[set("out", "kind", OUT_HIT)]),
];

/// Every `Start` sends one `Hit` on port 0, which the routes fan out.
#[must_use]
pub fn broadcaster() -> ProgramSpec {
    build(
        counter_base(),
        BROADCASTER_CONTRACTS,
        RefusalRule::SkipObservation,
    )
}

/// A broadcaster's table (the rally's `Hit` rule).
#[must_use]
pub fn broadcaster_table() -> EffectTable {
    rally_table()
}

macro_rules! seen {
    ($name:literal, $from:literal, $to:literal) => {
        contract($name)
            .when(&[signal(HIT)])
            .require(&[count($from)])
            .transition(&[set("count", "value", $to)])
    };
}

const SINK_CONTRACTS: &[ContractSpec] = &[
    seen!("seen-0", 0, 1),
    seen!("seen-1", 1, 2),
    seen!("seen-2", 2, 3),
    seen!("seen-3", 3, 4),
    seen!("seen-4", 4, 5),
    seen!("seen-5", 5, 6),
    seen!("seen-6", 6, 7),
    contract("full").when(&[signal(HIT)]).require(&[count(7)]),
];

/// Counts hits up to 7 and emits nothing. `Start` is refused (`NoContract`) and skipped.
#[must_use]
pub fn sink() -> ProgramSpec {
    let base = Program::new()
        .add(dimension!(Signal, "signal", Evidence))
        .add(dimension!(Count, "count", State));
    build(base, SINK_CONTRACTS, RefusalRule::SkipObservation)
}

/// The sink's contracts under `RefusalRule::Stop`: the first refusal halts it.
#[must_use]
pub fn strict_sink() -> ProgramSpec {
    let base = Program::new()
        .add(dimension!(Signal, "signal", Evidence))
        .add(dimension!(Count, "count", State));
    build(base, SINK_CONTRACTS, RefusalRule::Stop)
}

/// External tag of the mixed table's `External`.
pub const TAG_MIXED: u32 = 3;

/// An alarm whose `Arm` asks for four things at once: a `Hit` on port 0, two logical timers and
/// an external word. Each fits alone under one free timer place; together they do not.
#[must_use]
pub fn mixed_table() -> EffectTable {
    EffectTable::new()
        .on(
            "out",
            OUT_ARM,
            [
                send_hit(),
                logical_alarm(),
                logical_alarm(),
                RuntimeEffect::External {
                    tag: TAG_MIXED,
                    word: OUT_ARM,
                },
            ],
        )
        .on("out", OUT_RING, [ring()])
}

/// A volley's table whose `Hit` asks for one logical timer instead of a send: under
/// `RuntimeLimits { timers: 0, .. }` every step it prepares is refused (`TooManyTimers`), a
/// refusal that is not final, so `Backpressure` holds its first input for the whole run.
#[must_use]
pub fn stalled_table() -> EffectTable {
    EffectTable::new().on(
        "out",
        OUT_HIT,
        [RuntimeEffect::Timer {
            after: TickDelta(1),
            message: signal_message(HIT),
        }],
    )
}
