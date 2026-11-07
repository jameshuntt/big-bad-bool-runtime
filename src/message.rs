//! What crosses the runtime boundary between programs: evidence words by dimension name.

/// Evidence for the receiving program, by the receiver's dimension names.
///
/// A message holds only `'static` names and raw words. The receiver resolves each name against
/// its own spec when it steps; a name it does not have is handed to its stepper as a dimension it
/// does not have, and the stepper refuses it (`BBBC0108`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RuntimeMessage {
    evidence: Vec<(&'static str, u128)>,
}

impl RuntimeMessage {
    /// A message with no evidence.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            evidence: Vec::new(),
        }
    }

    /// Adds the word for one evidence dimension of the receiver.
    #[must_use]
    pub fn with(mut self, dimension: &'static str, word: u128) -> Self {
        self.evidence.push((dimension, word));
        self
    }

    /// Every `(dimension, word)` pair, in the order added.
    #[must_use]
    pub fn evidence(&self) -> &[(&'static str, u128)] {
        &self.evidence
    }
}
