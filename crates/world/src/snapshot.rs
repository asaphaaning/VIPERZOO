//! Publish coherent, immutable views of one projected world revision.
//!
//! A [`Snapshot`] is not an event log and does not permit mutation. It groups
//! related projection facets—map coverage, player location, entities, session
//! facts, and recent activity—after one complete reduction step. This lets a
//! script make a decision from internally consistent information while the
//! engine continues to receive observations elsewhere.
//!
//! Fields whose completeness matters expose it explicitly. For example,
//! inventory and equipment can contain known slots before their respective
//! authoritative scans establish a complete view.

use serde::Serialize;
use viperzoo_protocol::{
    combat, equipment, message,
    primitive::{EntityId, Position},
    spell, travel,
};

use crate::{action, entity, inventory, map, player, revision::Revision, session};

/// Current stable snapshot schema.
pub const SCHEMA_VERSION: &str = "0.12.0";

/// One immutable and internally consistent projected world.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Snapshot {
    schema_version: &'static str,
    revision: Revision,
    processed_packet_count: u64,
    unknown_packet_count: u64,
    map: map::Snapshot,
    player: player::State,
    entities: Vec<entity::State>,
    connection: session::Connection,
    heartbeat: session::Heartbeat,
    recent_actions: Vec<action::Event>,
    recent_combat_actions: Vec<action::Event>,
    spellbook: Vec<spell::Entry>,
    inventory: Vec<inventory::Item>,
    inventory_complete: bool,
    equipment: Vec<equipment::Item>,
    equipment_complete: bool,
    actor_actions: Vec<combat::Action>,
    vitality_events: Vec<combat::Vitality>,
    messages: Vec<message::Message>,
    travel_menu: Option<travel::Menu>,
}

impl Snapshot {
    pub(crate) fn new(
        revision: Revision,
        counts: Counts,
        core: CoreState,
        connection: session::Connection,
        heartbeat: session::Heartbeat,
        actions: Actions,
        server: ServerState,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision,
            processed_packet_count: counts.processed,
            unknown_packet_count: counts.unknown,
            map: core.map,
            player: core.player,
            entities: core.entities,
            connection,
            heartbeat,
            recent_actions: actions.recent,
            recent_combat_actions: actions.combat,
            spellbook: server.spellbook,
            inventory: server.possessions.inventory,
            inventory_complete: server.possessions.inventory_complete,
            equipment: server.possessions.equipment,
            equipment_complete: server.possessions.equipment_complete,
            actor_actions: server.actor_actions,
            vitality_events: server.vitality_events,
            messages: server.messages,
            travel_menu: server.travel_menu,
        }
    }

    /// Returns the canonical world revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the number of ordered packets presented to the reducer.
    #[must_use]
    pub const fn processed_packet_count(&self) -> u64 {
        self.processed_packet_count
    }

    /// Returns how many decoded packets remain unclassified.
    #[must_use]
    pub const fn unknown_packet_count(&self) -> u64 {
        self.unknown_packet_count
    }

    /// Returns active map knowledge and coverage.
    #[must_use]
    pub const fn map(&self) -> &map::Snapshot {
        &self.map
    }

    /// Returns projected player state.
    #[must_use]
    pub const fn player(&self) -> &player::State {
        &self.player
    }

    /// Borrows visible entities in stable identifier order.
    #[must_use]
    pub fn entities(&self) -> &[entity::State] {
        &self.entities
    }

    /// Finds one visible entity by stable identifier.
    #[must_use]
    pub fn entity(&self, id: EntityId) -> Option<&entity::State> {
        self.entities
            .binary_search_by_key(&id, entity::State::id)
            .ok()
            .map(|index| &self.entities[index])
    }

    /// Iterates visible entities at one coordinate.
    pub fn entities_at(&self, position: Position) -> impl Iterator<Item = &entity::State> {
        self.entities
            .iter()
            .filter(move |entity| entity.position() == position)
    }

    /// Iterates known floor items at one coordinate.
    pub fn ground_items_at(&self, position: Position) -> impl Iterator<Item = &entity::State> {
        self.entities_at(position)
            .filter(|entity| entity.appearance().is_floor_item())
    }

    /// Returns the projected client-session lifecycle.
    #[must_use]
    pub const fn connection(&self) -> session::Connection {
        self.connection
    }

    /// Returns projected heartbeat state.
    #[must_use]
    pub const fn heartbeat(&self) -> &session::Heartbeat {
        &self.heartbeat
    }

    /// Borrows recent client actions in observation order.
    #[must_use]
    pub fn recent_actions(&self) -> &[action::Event] {
        &self.recent_actions
    }

    /// Borrows recent client combat actions in observation order.
    #[must_use]
    pub fn recent_combat_actions(&self) -> &[action::Event] {
        &self.recent_combat_actions
    }

    /// Borrows projected spellbook slots in slot order.
    #[must_use]
    pub fn spellbook(&self) -> &[spell::Entry] {
        &self.spellbook
    }

    /// Borrows projected inventory slots in slot order.
    #[must_use]
    pub fn inventory(&self) -> &[inventory::Item] {
        &self.inventory
    }

    /// Returns whether an authoritative snapshot established every carried slot.
    #[must_use]
    pub const fn inventory_complete(&self) -> bool {
        self.inventory_complete
    }

    /// Borrows projected equipped items in equipment-slot order.
    #[must_use]
    pub fn equipment(&self) -> &[equipment::Item] {
        &self.equipment
    }

    /// Returns whether a full character profile established every equipment slot.
    #[must_use]
    pub const fn equipment_complete(&self) -> bool {
        self.equipment_complete
    }

    /// Borrows recent server-authored actor actions.
    #[must_use]
    pub fn actor_actions(&self) -> &[combat::Action] {
        &self.actor_actions
    }

    /// Borrows recent signed actor VITA effects.
    #[must_use]
    pub fn vitality_events(&self) -> &[combat::Vitality] {
        &self.vitality_events
    }

    /// Borrows recent server display messages.
    #[must_use]
    pub fn messages(&self) -> &[message::Message] {
        &self.messages
    }

    /// Returns the currently open server-provided travel menu, if any.
    #[must_use]
    pub const fn travel_menu(&self) -> Option<&travel::Menu> {
        self.travel_menu.as_ref()
    }
}

/// Core spatial and player projection used to construct one [`Snapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoreState {
    map: map::Snapshot,
    player: player::State,
    entities: Vec<entity::State>,
}

impl CoreState {
    pub(crate) const fn new(
        map: map::Snapshot,
        player: player::State,
        entities: Vec<entity::State>,
    ) -> Self {
        Self {
            map,
            player,
            entities,
        }
    }
}

/// Packet accounting used to construct one [`Snapshot`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Counts {
    processed: u64,
    unknown: u64,
}

impl Counts {
    pub(crate) const fn new(processed: u64, unknown: u64) -> Self {
        Self { processed, unknown }
    }
}

/// Bounded action histories used to construct one [`Snapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Actions {
    recent: Vec<action::Event>,
    combat: Vec<action::Event>,
}

impl Actions {
    pub(crate) const fn new(recent: Vec<action::Event>, combat: Vec<action::Event>) -> Self {
        Self { recent, combat }
    }
}

/// Server-authored script-facing state used to construct one [`Snapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServerState {
    spellbook: Vec<spell::Entry>,
    possessions: Possessions,
    actor_actions: Vec<combat::Action>,
    vitality_events: Vec<combat::Vitality>,
    messages: Vec<message::Message>,
    travel_menu: Option<travel::Menu>,
}

impl ServerState {
    pub(crate) const fn new(
        spellbook: Vec<spell::Entry>,
        possessions: Possessions,
        actor_actions: Vec<combat::Action>,
        vitality_events: Vec<combat::Vitality>,
        messages: Vec<message::Message>,
        travel_menu: Option<travel::Menu>,
    ) -> Self {
        Self {
            spellbook,
            possessions,
            actor_actions,
            vitality_events,
            messages,
            travel_menu,
        }
    }
}

/// Projected carried and equipped item state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Possessions {
    inventory: Vec<inventory::Item>,
    inventory_complete: bool,
    equipment: Vec<equipment::Item>,
    equipment_complete: bool,
}

impl Possessions {
    pub(crate) const fn new(
        inventory: Vec<inventory::Item>,
        inventory_complete: bool,
        equipment: Vec<equipment::Item>,
        equipment_complete: bool,
    ) -> Self {
        Self {
            inventory,
            inventory_complete,
            equipment,
            equipment_complete,
        }
    }
}
