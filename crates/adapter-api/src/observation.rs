//! Describe the complete facts an adapter may give the engine.
//!
//! [`Observation`] is intentionally broader than a packet stream. A fresh
//! attachment establishes a session boundary, a transport close happens below
//! plaintext decoding, and validated client memory can fill a late-attachment
//! gap. Keeping these cases in the same ordered vocabulary lets the reducer
//! apply their precedence and reset rules in one place.

use viperzoo_protocol::packet::Packet;

use crate::{inventory, resource};

/// One ordered fact from an attached client session.
#[derive(Debug)]
pub enum Observation {
    /// A new client attachment invalidates all session-scoped projection.
    SessionStarted,
    /// The client socket observed an orderly or local transport close.
    TransportClosed,
    /// A decoded plaintext protocol packet was observed.
    Packet(Packet),
    /// A validated late-attachment resource snapshot was read from the client.
    PlayerResources(resource::Resources),
    /// A validated complete carried-inventory snapshot was read from the client.
    PlayerInventory(inventory::Snapshot),
}

impl From<Packet> for Observation {
    fn from(packet: Packet) -> Self {
        Self::Packet(packet)
    }
}
