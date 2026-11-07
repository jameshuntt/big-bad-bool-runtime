//! What a committed step asks the runtime for, declared as data.
//!
//! A program's [`EffectTable`] maps one emitted effect word, by effect dimension name and word, to
//! the list of runtime effects it asks for. It is data only: built by the caller, resolved
//! against the program's spec once at `build`, and read by lookup. No code of the caller runs
//! inside a poll, so the actions of a step are a function of its effect words alone: there is no
//! clock, randomness, environment or mutable global anywhere on that path.

use std::collections::BTreeMap;

use big_bad_bool::program::{DimensionRole, Effect, ProgramSpec};
use scope::TickDelta;

use crate::ids::{ChildKey, Port};
use crate::message::RuntimeMessage;

/// One request a committed step makes of the runtime. Admission checks every request of the
/// step before the step commits; the runtime acts on them only after it committed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuntimeEffect {
    /// Deliver `message` to every program routed from this program's `port`, in route order.
    Send {
        /// The outgoing port.
        port: Port,
        /// The evidence for each receiver.
        message: RuntimeMessage,
    },
    /// Deliver `message` back to this program `after` scheduler rounds, on Scope's logical clock.
    /// There is no wall-clock timer: real time is outside replay and outside this crate.
    Timer {
        /// Rounds after the timer task's first poll, the round after the step (Scope's
        /// `TickSleepTask`).
        after: TickDelta,
        /// The evidence delivered.
        message: RuntimeMessage,
    },
    /// Start a scoped child activity of this program.
    StartChild {
        /// The child's name inside this program.
        key: ChildKey,
        /// What the child does.
        child: ChildSpec,
    },
    /// Cancel this program's child `key` through Scope's cancellation.
    CancelChild {
        /// The child's name inside this program.
        key: ChildKey,
    },
    /// Hand a word to the application outside the runtime.
    External {
        /// The application's tag.
        tag: u32,
        /// The word.
        word: u128,
    },
    /// A mark in the run's log; changes nothing.
    Trace {
        /// The mark.
        tag: u32,
    },
}

/// A child activity. Scope owns its lifetime: it is a Scope task, spawned by the parent's
/// committed step and ended by completion or by Scope's cancellation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChildSpec {
    /// Delivers `message` to the parent once per scheduler round, `ticks` times.
    Ticker {
        /// How many deliveries before it completes on its own.
        ticks: u32,
        /// The evidence delivered each time.
        message: RuntimeMessage,
    },
}

/// One declared rule: the effect word `word` on effect dimension `dimension` asks for `actions`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectRule {
    /// The effect dimension's name in the program's spec.
    pub dimension: &'static str,
    /// The whole effect word.
    pub word: u128,
    /// What it asks for, in order.
    pub actions: Vec<RuntimeEffect>,
}

/// A program's effect mapping, as data. An effect word with no rule asks for nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct EffectTable {
    rules: Vec<EffectRule>,
}

impl EffectTable {
    /// A table with no rules: the program's effects stay inside it.
    #[must_use]
    pub const fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Adds the rule `dimension = word` asks for `actions`.
    #[must_use]
    pub fn on(
        mut self,
        dimension: &'static str,
        word: u128,
        actions: impl IntoIterator<Item = RuntimeEffect>,
    ) -> Self {
        self.rules.push(EffectRule {
            dimension,
            word,
            actions: actions.into_iter().collect(),
        });
        self
    }

    /// The rules, in the order declared.
    #[must_use]
    pub fn rules(&self) -> &[EffectRule] {
        &self.rules
    }
}

/// Why a table does not fit its program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    /// The spec has no dimension of this name.
    UnknownDimension(&'static str),
    /// The dimension is not an effect dimension.
    NotAnEffect(&'static str),
    /// Two rules for one `(dimension, word)`.
    Duplicate {
        /// The dimension.
        dimension: &'static str,
        /// The word.
        word: u128,
    },
}

/// A table resolved against one spec: `(dimension id, word)` to actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTable {
    map: BTreeMap<(u16, u128), Vec<RuntimeEffect>>,
}

impl ResolvedTable {
    pub(crate) fn resolve(spec: &ProgramSpec, table: &EffectTable) -> Result<Self, TableError> {
        let mut map = BTreeMap::new();
        for rule in &table.rules {
            let id = spec
                .dimension_id(rule.dimension)
                .ok_or(TableError::UnknownDimension(rule.dimension))?;
            let effect = spec
                .dimension(id)
                .is_some_and(|d| d.role() == DimensionRole::Effect);
            if !effect {
                return Err(TableError::NotAnEffect(rule.dimension));
            }
            if map
                .insert((id.0, rule.word), rule.actions.clone())
                .is_some()
            {
                return Err(TableError::Duplicate {
                    dimension: rule.dimension,
                    word: rule.word,
                });
            }
        }
        Ok(Self { map })
    }

    /// The actions one effect word asks for.
    pub(crate) fn actions(&self, effect: &Effect) -> &[RuntimeEffect] {
        self.map
            .get(&(effect.dimension.0, effect.word))
            .map_or(&[], Vec::as_slice)
    }
}
