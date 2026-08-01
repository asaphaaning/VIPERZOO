//! Spellbook state sent by the server.

use serde::Serialize;

use crate::primitive::Opaque;

/// One learned spellbook slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Entry {
    slot: u8,
    kind: u8,
    name: String,
    question: String,
    opaque_tail: Opaque,
}

impl Entry {
    pub(crate) fn new(slot: u8, kind: u8, name: String, question: String, tail: &[u8]) -> Self {
        Self {
            slot,
            kind,
            name,
            question,
            opaque_tail: Opaque::from_slice(tail),
        }
    }

    /// Returns the one-based spellbook slot.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }
    /// Returns the spell targeting/type byte.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }
    /// Returns the display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the prompt used by question/target spells.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }
}
