//! Recent client action observations.

use serde::Serialize;
use viperzoo_protocol::{client, direction::Direction, primitive::Position};

use crate::revision::Revision;

/// Maximum recent client actions retained in one snapshot.
pub const CAPACITY: usize = 32;

/// Maximum client combat actions retained independently of general activity.
pub const COMBAT_CAPACITY: usize = 128;

/// One typed client action at a canonical world revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Event {
    revision: Revision,
    action: Action,
}

impl Event {
    /// Converts a client packet when it represents a player action.
    #[must_use]
    pub fn from_packet(packet: &client::Packet, revision: Revision) -> Option<Self> {
        let action = match packet {
            client::Packet::Speech(speech) => Action::Speak {
                text: speech.text().into(),
            },
            client::Packet::Movement(movement) => Action::Step {
                direction: movement.direction(),
                origin: movement.origin(),
                last_walk: movement.last_walk(),
            },
            client::Packet::Obstruction(obstruction) => Action::Obstruction {
                origin: obstruction.origin(),
                direction: obstruction.direction(),
            },
            client::Packet::Facing(facing) => Action::Face {
                direction: facing.direction(),
            },
            client::Packet::Attack(_) => Action::Attack,
            client::Packet::Pickup(_) => Action::Pickup,
            client::Packet::Refresh(_) => Action::Refresh,
            client::Packet::Disconnect(_) => Action::Disconnect,
            client::Packet::UseInventory(item) => Action::UseInventory { slot: item.slot() },
            client::Packet::Cast(cast) => Action::Cast { slot: cast.slot() },
            client::Packet::Interact(interact) => Action::Interact {
                entity: interact.entity(),
            },
            client::Packet::Dialog(dialog) => Action::Dialog {
                entity: dialog.entity(),
                command: dialog.command(),
            },
            client::Packet::TravelSelection(selection) => Action::TravelSelection {
                map: selection.map(),
                position: selection.position(),
            },
            client::Packet::Heartbeat(_) | client::Packet::Unknown(_) => return None,
        };

        Some(Self { revision, action })
    }

    /// Returns the world revision that observed the action.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the typed action.
    #[must_use]
    pub const fn action(&self) -> &Action {
        &self.action
    }

    /// Returns whether this event is a client-authored combat request.
    #[must_use]
    pub const fn is_combat(&self) -> bool {
        matches!(self.action, Action::Attack | Action::Cast { .. })
    }
}

/// Player-authored action vocabulary currently promoted from live evidence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// A one-tile movement request.
    Step {
        /// Requested direction.
        direction: Direction,
        /// Position before applying the step.
        origin: Position,
        /// Wrapping movement counter.
        last_walk: u8,
    },
    /// A direction-only turn.
    Face {
        /// Requested direction.
        direction: Direction,
    },
    /// An ordinary attack in the current facing direction.
    Attack,
    /// Pick up an item from the current tile.
    Pickup,
    /// Request a visible-map redraw and resynchronization.
    Refresh,
    /// The client emitted its canonical close body.
    ///
    /// This wire observation does not distinguish operator exit, local idle
    /// closure, or a close handshake following a server-initiated boot.
    Disconnect,
    /// Activate an item from a one-based inventory slot.
    UseInventory {
        /// One-based inventory slot.
        slot: u8,
    },
    /// A movement edge rejected by the native client's collision state.
    Obstruction {
        /// Unchanged player tile.
        origin: Position,
        /// Rejected direction.
        direction: Direction,
    },
    /// A spell invocation by one-based spellbook slot.
    Cast {
        /// One-based spellbook slot.
        slot: u8,
    },
    /// Submit a public or NPC-directed speech line.
    Speak {
        /// The exact client-authored text.
        text: Box<str>,
    },
    /// Open or advance interaction with a visible NPC or actor.
    Interact {
        /// Target entity.
        entity: viperzoo_protocol::primitive::EntityId,
    },
    /// Submit a server-authored NPC dialog command.
    Dialog {
        /// Target NPC.
        entity: viperzoo_protocol::primitive::EntityId,
        /// Server-authored command token.
        command: u8,
    },
    /// Choose one destination from a travel menu.
    TravelSelection {
        /// Selected map.
        map: viperzoo_protocol::primitive::MapId,
        /// Entry coordinate within the selected map.
        position: Position,
    },
}
