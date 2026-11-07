//! Identifiers.
//!
//! Durable ids are `uuid_macros::typed_uuid!` newtypes made only by name (UUID v5), each kind
//! under its own namespace through `named_v5`, so the same named topology gives the same ids in
//! every run and on every machine, and ids of two kinds never collide. Inside a run, programs are
//! dense [`ProgramSlot`]s and messages are numbers. The runtime never makes a random (v4) id:
//! `typed_uuid!` also generates `new_v4` and an OID-namespace `named`, and neither is called here.

use std::fmt;

/// The namespace of every [`ProgramId`] made by [`ProgramId::from_name`].
pub const PROGRAM_NAMESPACE: &str = "big-bad-bool-runtime/program";
/// The namespace of every [`RouteId`].
pub const ROUTE_NAMESPACE: &str = "big-bad-bool-runtime/route";
/// The namespace of every [`TimerId`].
pub const TIMER_NAMESPACE: &str = "big-bad-bool-runtime/timer";
/// The namespace of every [`RunId`].
pub const RUN_NAMESPACE: &str = "big-bad-bool-runtime/run";

/// The durable id types, declared by `uuid_macros::typed_uuid!`.
#[allow(missing_docs)]
mod durable {
    uuid_macros::typed_uuid!(ProgramId, "program");
    uuid_macros::typed_uuid!(RouteId, "route");
    uuid_macros::typed_uuid!(RunId, "run");
    uuid_macros::typed_uuid!(TimerId, "timer");
}

pub use durable::{ProgramId, RouteId, RunId, TimerId};

impl ProgramId {
    /// The program named `name`: v5 of `name` under [`PROGRAM_NAMESPACE`].
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        Self::named_v5(PROGRAM_NAMESPACE, name)
    }
}

impl RouteId {
    /// The route from `from`'s `port` to `to`: v5 of `"<from>/<port>/<to>"` under
    /// [`ROUTE_NAMESPACE`].
    #[must_use]
    pub fn of(from: ProgramId, port: Port, to: ProgramId) -> Self {
        Self::named_v5(ROUTE_NAMESPACE, &format!("{from}/{}/{to}", port.0))
    }
}

impl TimerId {
    /// The `ordinal`-th timer (from 0) armed by `program` in a run: v5 of `"<program>/<ordinal>"`
    /// under [`TIMER_NAMESPACE`]. The same run arms the same timers under the same ids.
    #[must_use]
    pub fn of(program: ProgramId, ordinal: u32) -> Self {
        Self::named_v5(TIMER_NAMESPACE, &format!("{program}/{ordinal}"))
    }
}

impl RunId {
    /// The run of `topology` (the text [`crate::Runtime::run_id`] describes): v5 under
    /// [`RUN_NAMESPACE`].
    #[must_use]
    pub fn of_topology(topology: &str) -> Self {
        Self::named_v5(RUN_NAMESPACE, topology)
    }
}

/// A program's place in a run: its registration index. Dense, so the run loop and the trace index
/// by it; [`crate::RunOutcome::program_ids`] maps it back to the [`ProgramId`]. No API takes a
/// slot as input: programs are named by [`ProgramId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramSlot(pub u32);

/// An outgoing port of a program. Routes bind `(program, port)` to receiving programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Port(pub u8);

/// A child activity of one program, named by the program's own effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChildKey(pub u8);

/// One message, numbered in the order the runtime created it during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(pub u64);

/// Who created a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Injected through the builder.
    External,
    /// A committed step of this program, through a route.
    Program(ProgramSlot),
    /// A timer this program armed.
    Timer(ProgramSlot, TimerId),
    /// A child activity of this program.
    Child(ProgramSlot, ChildKey),
}

impl fmt::Display for ProgramSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "slot{}", self.0)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m{}", self.0)
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn slot(index: usize) -> ProgramSlot {
    ProgramSlot(index as u32)
}
