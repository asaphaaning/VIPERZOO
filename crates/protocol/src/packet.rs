//! Preserve packet direction and incompletely understood payloads.
//!
//! [`Packet`] makes clientbound and serverbound bodies different at the type
//! boundary, preventing a reducer from accidentally treating one as the other.
//! [`Unknown`] keeps an unpromoted opcode and its complete logical body intact;
//! it is evidence for protocol research, not a malformed packet or discarded
//! input.

use serde::Serialize;

use crate::primitive::{Body, Opaque};
use crate::{client, server};

/// A decoded plaintext logical body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "flow", content = "packet", rename_all = "snake_case")]
pub enum Packet {
    /// A body sent from the server to the client.
    Clientbound(server::Packet),
    /// A body sent from the client to the server.
    Serverbound(client::Packet),
}

/// An opcode whose semantics are not yet promoted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Unknown {
    opcode: u8,
    body: Body,
}

impl Unknown {
    pub(crate) fn new(opcode: u8, body: &[u8]) -> Self {
        Self {
            opcode,
            body: Body::from_slice(body),
        }
    }

    /// Returns the unclassified opcode.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.opcode
    }

    /// Returns the untouched logical body.
    #[must_use]
    pub const fn body(&self) -> &Body {
        &self.body
    }
}

/// Common access to retained workspace bytes.
pub trait HasOpaqueTail {
    /// Returns bytes outside the currently understood semantic layout.
    fn opaque_tail(&self) -> &Opaque;
}
