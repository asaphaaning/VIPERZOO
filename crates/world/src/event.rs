//! Describe non-coalescing edges in the canonical world timeline.
//!
//! A [`crate::snapshot::Snapshot`] answers “what is true now?” and may safely
//! coalesce intermediate revisions. An [`Event`] answers “what happened at
//! this revision?” and must therefore be delivered in order or report a gap.
//! The engine owns that delivery policy; this module owns the closed domain
//! vocabulary attached to each revision.

use serde::Serialize;
use viperzoo_adapter_api::observation::Observation;
use viperzoo_protocol::{client, combat, direction::Flow, message, packet, server};

use crate::{action, revision::Revision, world::Change};

/// One canonical edge produced by reducing an ordered observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Event {
    change: Change,
    kind: Kind,
}

impl Event {
    /// Creates a canonical edge from its projection effect and typed fact.
    #[must_use]
    pub const fn new(change: Change, kind: Kind) -> Self {
        Self { change, kind }
    }

    /// Promotes one boundary observation into the canonical event vocabulary.
    #[must_use]
    pub fn from_observation(observation: &Observation, change: Change) -> Self {
        Self::new(change, Kind::from_observation(observation))
    }

    /// Returns the resulting world revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.change.revision()
    }

    /// Returns the projection effect of this edge.
    #[must_use]
    pub const fn change(&self) -> Change {
        self.change
    }

    /// Returns the typed fact recorded at this edge.
    #[must_use]
    pub const fn kind(&self) -> &Kind {
        &self.kind
    }
}

/// Closed vocabulary of facts that can advance the canonical world.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Kind {
    /// A new adapter attachment began a fresh session epoch.
    SessionStarted,
    /// The active client transport closed below the packet boundary.
    TransportClosed,
    /// Map identity, environment, or tile coverage was observed.
    Map,
    /// Player location or status was observed.
    Player,
    /// Validated player resource values were read from client memory.
    Resources,
    /// Spellbook state was observed.
    Spellbook,
    /// Carried inventory state was observed.
    Inventory,
    /// Equipped item state was observed.
    Equipment,
    /// Visible entity state was observed.
    Entities,
    /// A player-authored action was observed on the client transport.
    ClientAction(action::Action),
    /// A server-authored actor animation was observed.
    ActorAction(combat::Action),
    /// A server-authored actor VITA effect was observed.
    ActorVitality(combat::Vitality),
    /// A server display message was observed.
    Message(message::Message),
    /// A server-provided travel menu was observed.
    Travel,
    /// Heartbeat evidence was observed in either direction.
    Heartbeat,
    /// An unclassified packet was retained without inventing semantics.
    UnknownPacket {
        /// Direction in which the unknown body was observed.
        flow: Flow,
    },
}

impl Kind {
    /// Promotes one boundary observation into its canonical fact category.
    #[must_use]
    pub fn from_observation(observation: &Observation) -> Self {
        match observation {
            Observation::SessionStarted => Self::SessionStarted,
            Observation::TransportClosed => Self::TransportClosed,
            Observation::Packet(packet) => Self::from_packet(packet),
            Observation::PlayerResources(_) => Self::Resources,
            Observation::PlayerInventory(_) => Self::Inventory,
            Observation::ClientMap { .. } => Self::Map,
        }
    }

    fn from_packet(packet: &packet::Packet) -> Self {
        match packet {
            packet::Packet::Clientbound(packet) => Self::from_clientbound(packet),
            packet::Packet::Serverbound(packet) => Self::from_serverbound(packet),
        }
    }

    fn from_clientbound(packet: &server::Packet) -> Self {
        match packet {
            server::Packet::MapContext(_) | server::Packet::MapRegion(_) => Self::Map,
            server::Packet::PlayerLocation(_) | server::Packet::PlayerStatus(_) => Self::Player,
            server::Packet::SpellbookEntry(_) => Self::Spellbook,
            server::Packet::InventoryItem(_) | server::Packet::InventoryCleared(_) => {
                Self::Inventory
            }
            server::Packet::EquipmentItem(_)
            | server::Packet::EquipmentCleared(_)
            | server::Packet::CharacterProfile(_) => Self::Equipment,
            server::Packet::ActorAction(action) => Self::ActorAction(action.clone()),
            server::Packet::ActorVitality(vitality) => Self::ActorVitality(vitality.clone()),
            server::Packet::Message(message) => Self::Message(message.clone()),
            server::Packet::TravelMenu(_) => Self::Travel,
            server::Packet::EntityAppearances(_)
            | server::Packet::EntityMovement(_)
            | server::Packet::EntityRemoval(_)
            | server::Packet::EntityControl(_) => Self::Entities,
            server::Packet::Heartbeat(_) => Self::Heartbeat,
            server::Packet::Unknown(_) => Self::UnknownPacket {
                flow: Flow::Clientbound,
            },
        }
    }

    fn from_serverbound(packet: &client::Packet) -> Self {
        match action::classify(packet) {
            action::Packet::Action(action) => Self::ClientAction(action),
            action::Packet::Heartbeat => Self::Heartbeat,
            action::Packet::Unknown => Self::UnknownPacket {
                flow: Flow::Serverbound,
            },
        }
    }
}
