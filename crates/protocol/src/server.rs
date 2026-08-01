//! Server-to-client packet vocabulary.

use serde::Serialize;

use crate::{
    combat, entity, equipment, heartbeat, inventory, map, message, packet, player, profile, spell,
    travel,
};

/// A decoded server-to-client plaintext body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Packet {
    /// Active map identity and environment.
    MapContext(map::Context),
    /// Static tile rectangle or scrolling strip.
    MapRegion(map::Region),
    /// Player localization observation.
    PlayerLocation(player::Location),
    /// Composable player status blocks.
    PlayerStatus(player::Status),
    /// One learned spellbook slot.
    SpellbookEntry(spell::Entry),
    /// One inventory slot initialization or update.
    InventoryItem(inventory::Item),
    /// One inventory slot became empty.
    InventoryCleared(inventory::Cleared),
    /// One equipped item initialization or replacement.
    EquipmentItem(equipment::Item),
    /// One equipped slot became empty.
    EquipmentCleared(equipment::Cleared),
    /// Detailed character sheet used to reconcile the complete equipment set.
    CharacterProfile(profile::Character),
    /// Actor animation/action state.
    ActorAction(combat::Action),
    /// Actor health-bar damage or healing effect.
    ActorVitality(combat::Vitality),
    /// Client display message.
    Message(message::Message),
    /// Server travel destination menu became available.
    TravelMenu(travel::Menu),
    /// One or more newly visible entities.
    EntityAppearances(entity::Appearances),
    /// One stable entity moved one tile.
    EntityMovement(entity::Movement),
    /// One stable entity left visibility.
    EntityRemoval(entity::Removal),
    /// Unresolved visibility control observed after a fresh appearance batch.
    EntityControl(entity::Control),
    /// Server heartbeat challenge.
    Heartbeat(heartbeat::Ping),
    /// An unclassified body retained without loss.
    Unknown(packet::Unknown),
}
