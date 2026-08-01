//! Length-prefixed server messages.

use serde::Serialize;

use crate::primitive::Opaque;

/// One message displayed by the client.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Message {
    kind: u8,
    text: String,
    opaque_tail: Opaque,
}

impl Message {
    pub(crate) fn new(kind: u8, text: String, tail: &[u8]) -> Self {
        Self {
            kind,
            text,
            opaque_tail: Opaque::from_slice(tail),
        }
    }
    /// Returns the display/message type.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }
    /// Returns decoded client text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}
