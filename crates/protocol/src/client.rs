//! Client-to-server packet vocabulary.

use serde::Serialize;

use crate::{action, heartbeat, packet};

/// A decoded client-to-server plaintext body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Packet {
    /// One client-authored speech line.
    Speech(action::Speech),
    /// One client movement request.
    Movement(action::Movement),
    /// One rejected client movement edge.
    Obstruction(action::Obstruction),
    /// One direction-only facing request.
    Facing(action::Facing),
    /// One ordinary attack request.
    Attack(action::Attack),
    /// One ground-item pickup request.
    Pickup(action::Pickup),
    /// One client-emitted session close handshake.
    Disconnect(action::Disconnect),
    /// One inventory-slot activation request.
    UseInventory(action::UseInventory),
    /// One spell invocation request.
    Cast(action::Cast),
    /// One visible-NPC interaction request.
    Interact(action::Interact),
    /// One structured response in an NPC dialog.
    Dialog(action::Dialog),
    /// One destination selection from a travel menu.
    TravelSelection(action::TravelSelection),
    /// One visible-map refresh request.
    Refresh(action::Refresh),
    /// Client heartbeat response.
    Heartbeat(heartbeat::Pong),
    /// An unclassified body retained without loss.
    Unknown(packet::Unknown),
}
