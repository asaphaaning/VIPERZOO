//! Reduce ordered observations while preserving projection invariants.
//!
//! [`World`] is deliberately smaller than the engine that owns it. It has no
//! channels, tasks, or adapter details—only the state transition rules that
//! turn one validated observation into a new revision. Packet facts take
//! precedence over late-attachment memory seeds, and session starts reset only
//! session-scoped state.
//!
//! [`Change::Recorded`] means evidence advanced without changing a script-facing
//! fact. [`Change::Projected`] means a snapshot facet changed. Both advance the
//! revision, allowing consumers to distinguish ordering from meaningful state
//! change without losing either.

use std::collections::{BTreeMap, VecDeque};

use tracing::instrument;
use viperzoo_adapter_api::{inventory as adapter_inventory, resource};
use viperzoo_protocol::{client, map as protocol, packet, server};

use crate::{
    action, entity, inventory, map, player,
    revision::Revision,
    session,
    snapshot::{self, Snapshot},
};

/// The effect of recording one decoded packet.
///
/// Both variants advance the [`Revision`]; they differ in whether a script
/// watching the world would see anything new. Collapsing them into a boolean
/// would lose exactly the distinction a subscriber needs — re-rendering on every
/// heartbeat is waste, and treating a quiet engine as a stalled one is a bug.
///
/// ```text
/// heartbeat pong      ──► Recorded   evidence advanced, projection identical
/// player stepped      ──► Projected  a script-visible fact changed
/// ```
///
/// ```
/// use viperzoo_world::world::{Change, World};
/// use viperzoo_world::revision::Revision;
///
/// let mut world = World::new();
/// let change = world.observe_transport_close();
///
/// assert!(change.is_projected());
/// assert!(change.revision() > Revision::INITIAL);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    /// Ordering/evidence changed without changing a projected domain facet.
    Recorded(Revision),
    /// At least one projected domain facet changed.
    Projected(Revision),
}

impl Change {
    /// Returns the resulting canonical revision.
    #[must_use]
    pub const fn revision(self) -> Revision {
        match self {
            Self::Recorded(revision) | Self::Projected(revision) => revision,
        }
    }

    /// Returns whether a domain projection changed.
    #[must_use]
    pub const fn is_projected(self) -> bool {
        matches!(self, Self::Projected(_))
    }
}

/// The canonical, single-writer projected world.
#[derive(Debug, Default)]
pub struct World {
    revision: Revision,
    processed_packet_count: u64,
    unknown_packet_count: u64,
    map: map::State,
    player: player::State,
    entities: BTreeMap<viperzoo_protocol::primitive::EntityId, entity::State>,
    connection: session::Connection,
    heartbeat: session::Heartbeat,
    recent_actions: VecDeque<action::Event>,
    recent_combat_actions: VecDeque<action::Event>,
    spellbook: BTreeMap<u8, viperzoo_protocol::spell::Entry>,
    inventory: BTreeMap<u8, inventory::Item>,
    inventory_complete: bool,
    equipment: BTreeMap<u8, viperzoo_protocol::equipment::Item>,
    equipment_complete: bool,
    actor_actions: VecDeque<viperzoo_protocol::combat::Action>,
    vitality_events: VecDeque<viperzoo_protocol::combat::Vitality>,
    messages: VecDeque<viperzoo_protocol::message::Message>,
    travel_menu: Option<viperzoo_protocol::travel::Menu>,
}

impl World {
    /// Creates an empty warm-attachment-capable world.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records and reduces one decoded packet in order.
    #[instrument(
        name = "viperzoo::world::apply_packet",
        skip(self, packet),
        ret(level = "debug")
    )]
    pub fn apply(&mut self, packet: &packet::Packet) -> Change {
        let revision = self.revision.next();
        self.processed_packet_count += 1;

        let projected = match packet {
            packet::Packet::Clientbound(packet) => self.apply_clientbound(packet, revision),
            packet::Packet::Serverbound(packet) => self.apply_serverbound(packet, revision),
        };

        self.revision = revision;

        if projected {
            Change::Projected(revision)
        } else {
            Change::Recorded(revision)
        }
    }

    /// Establishes map identity read from the client's own model.
    ///
    /// A warm attachment misses the `0x15` context that names the map, leaving
    /// routing to infer position without knowing which map the coordinates
    /// belong to. This fills that gap only: an identity the server described is
    /// never replaced by one read from memory.
    #[instrument(
        name = "viperzoo::world::seed_map_identity",
        skip(self),
        fields(map = identity.id().value()),
        ret(level = "debug")
    )]
    pub fn seed_map_identity(
        &mut self,
        identity: protocol::Identity,
        title: Option<String>,
    ) -> Change {
        let revision = self.revision.next();
        let transition = self.map.observe_identity(identity, title);

        self.revision = revision;

        if matches!(transition, map::Transition::Unchanged) {
            Change::Recorded(revision)
        } else {
            Change::Projected(revision)
        }
    }

    /// Merges a validated late-attachment resource snapshot.
    ///
    /// Packet-derived fields retain precedence over memory-derived fields.
    #[instrument(
        name = "viperzoo::world::seed_resources",
        skip(self, resources),
        ret(level = "debug")
    )]
    pub fn seed_resources(&mut self, resources: resource::Resources) -> Change {
        let revision = self.revision.next();
        let projected = self.player.seed_resources(resources, revision);

        self.revision = revision;

        if projected {
            Change::Projected(revision)
        } else {
            Change::Recorded(revision)
        }
    }

    /// Replaces carried inventory with one validated complete client scan.
    #[instrument(
        name = "viperzoo::world::seed_inventory",
        skip(self, snapshot),
        fields(capacity = snapshot.capacity(), occupied = snapshot.items().len()),
        ret(level = "debug")
    )]
    pub fn seed_inventory(&mut self, snapshot: &adapter_inventory::Snapshot) -> Change {
        let revision = self.revision.next();
        let inventory = snapshot
            .items()
            .iter()
            .map(inventory::Item::from_client)
            .map(|item| (item.slot(), item))
            .collect::<BTreeMap<_, _>>();
        let projected = !self.inventory_complete || self.inventory != inventory;

        self.inventory = inventory;
        self.inventory_complete = true;
        self.revision = revision;

        if projected {
            Change::Projected(revision)
        } else {
            Change::Recorded(revision)
        }
    }

    /// Creates an immutable, internally consistent snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        snapshot::Snapshot::new(
            self.revision,
            snapshot::Counts::new(self.processed_packet_count, self.unknown_packet_count),
            snapshot::CoreState::new(
                self.map.snapshot(),
                self.player.clone(),
                self.entities.values().cloned().collect(),
            ),
            self.connection,
            self.heartbeat.clone(),
            snapshot::Actions::new(
                self.recent_actions.iter().cloned().collect(),
                self.recent_combat_actions.iter().cloned().collect(),
            ),
            snapshot::ServerState::new(
                self.spellbook.values().cloned().collect(),
                snapshot::Possessions::new(
                    self.inventory.values().cloned().collect(),
                    self.inventory_complete,
                    self.equipment.values().cloned().collect(),
                    self.equipment_complete,
                ),
                self.actor_actions.iter().cloned().collect(),
                self.vitality_events.iter().cloned().collect(),
                self.messages.iter().cloned().collect(),
                self.travel_menu.clone(),
            ),
        )
    }

    fn apply_clientbound(&mut self, packet: &server::Packet, revision: Revision) -> bool {
        match packet {
            server::Packet::MapContext(context) => {
                let transition = self.map.observe_context(context);
                self.travel_menu = None;

                if transition.invalidates_scoped_state() {
                    self.entities.clear();
                    let _ = self.player.clear_location();
                }

                transition.changed()
            }
            server::Packet::MapRegion(region) => self.map.merge(region),
            server::Packet::PlayerLocation(location) => {
                self.player.observe_location(location, revision)
            }
            server::Packet::PlayerStatus(status) => self.player.observe_status(status, revision),
            server::Packet::SpellbookEntry(entry) => {
                self.spellbook.insert(entry.slot(), entry.clone()).as_ref() != Some(entry)
            }
            server::Packet::InventoryItem(item) => {
                let item = inventory::Item::from_packet(item);

                self.inventory.insert(item.slot(), item.clone()).as_ref() != Some(&item)
            }
            server::Packet::InventoryCleared(clear) => {
                self.inventory.remove(&clear.slot()).is_some()
            }
            server::Packet::EquipmentItem(item) => {
                self.equipment.insert(item.slot(), item.clone()).as_ref() != Some(item)
            }
            server::Packet::EquipmentCleared(clear) => {
                self.equipment.remove(&clear.slot()).is_some()
            }
            server::Packet::CharacterProfile(profile) => {
                let equipment = profile
                    .equipment()
                    .iter()
                    .map(|item| (item.slot(), item.clone()))
                    .collect::<BTreeMap<_, _>>();
                let changed = !self.equipment_complete || self.equipment != equipment;

                self.equipment = equipment;
                self.equipment_complete = true;
                changed
            }
            server::Packet::ActorAction(action) => {
                push_bounded(&mut self.actor_actions, action.clone(), 128);
                true
            }
            server::Packet::ActorVitality(vitality) => {
                if let Some(entity) = self.entities.get_mut(&vitality.actor()) {
                    let _ = entity.observe_vitality(vitality, revision);
                }
                push_bounded(&mut self.vitality_events, vitality.clone(), 128);
                true
            }
            server::Packet::Message(message) => {
                push_bounded(&mut self.messages, message.clone(), 32);
                true
            }
            server::Packet::TravelMenu(menu) => {
                let changed = self.travel_menu.as_ref() != Some(menu);
                self.travel_menu = Some(menu.clone());
                changed
            }
            server::Packet::EntityAppearances(appearances) => {
                let mut changed = false;

                for appearance in appearances.entities() {
                    let previous = self.entities.get(&appearance.id());
                    let projected = entity::State::appeared(appearance, previous, revision);

                    changed |= previous.is_none_or(|previous| !previous.same_value(&projected));
                    self.entities.insert(appearance.id(), projected);
                }

                changed
            }
            server::Packet::EntityMovement(movement) => {
                let previous = self.entities.get(&movement.id());
                let Some(projected) = entity::State::moved(previous, movement, revision) else {
                    return false;
                };
                let changed = previous.is_none_or(|previous| !previous.same_value(&projected));

                self.entities.insert(movement.id(), projected);
                changed
            }
            server::Packet::EntityRemoval(removal) => self.entities.remove(&removal.id()).is_some(),
            // Captured refresh ordering is 0x07 appearances followed by 0x58.
            // Clearing here erased the server's newly published viewport. Map
            // context changes already invalidate map-scoped entities safely.
            server::Packet::EntityControl(_) => false,
            server::Packet::Heartbeat(ping) => {
                self.heartbeat.observe_ping(ping);
                true
            }
            server::Packet::Unknown(_) => {
                self.unknown_packet_count += 1;
                false
            }
        }
    }

    fn apply_serverbound(&mut self, packet: &client::Packet, revision: Revision) -> bool {
        let mut projected = false;

        if let Some(action) = action::Event::from_packet(packet, revision) {
            if action.is_combat() {
                if self.recent_combat_actions.len() == action::COMBAT_CAPACITY {
                    let _ = self.recent_combat_actions.pop_front();
                }

                self.recent_combat_actions.push_back(action.clone());
            }

            if self.recent_actions.len() == action::CAPACITY {
                let _ = self.recent_actions.pop_front();
            }

            self.recent_actions.push_back(action);
            projected = true;
        }

        projected
            | match packet {
                client::Packet::Movement(movement) => {
                    self.player.observe_movement(movement, revision)
                }
                client::Packet::Obstruction(obstruction) => {
                    self.player.observe_obstruction(*obstruction, revision)
                }
                client::Packet::Facing(facing) => self.player.observe_facing(*facing, revision),
                client::Packet::Attack(_)
                | client::Packet::Pickup(_)
                | client::Packet::Refresh(_)
                | client::Packet::UseInventory(_)
                | client::Packet::Cast(_)
                | client::Packet::Speech(_)
                | client::Packet::Interact(_)
                | client::Packet::Dialog(_)
                | client::Packet::TravelSelection(_) => false,
                client::Packet::Disconnect(_) => {
                    self.connection = session::Connection::CloseObserved;
                    true
                }
                client::Packet::Heartbeat(pong) => {
                    self.heartbeat.observe_pong(pong);
                    true
                }
                client::Packet::Unknown(_) => {
                    self.unknown_packet_count += 1;
                    false
                }
            }
    }

    /// Starts a new client session while retaining the monotonic world revision.
    #[instrument(
        name = "viperzoo::world::begin_session",
        skip(self),
        ret(level = "debug")
    )]
    pub fn begin_session(&mut self) -> Change {
        let revision = self.revision.next();

        self.processed_packet_count = 0;
        self.unknown_packet_count = 0;
        self.map = map::State::default();
        self.player = player::State::default();
        self.entities.clear();
        self.connection = session::Connection::Active;
        self.heartbeat = session::Heartbeat::default();
        self.recent_actions.clear();
        self.recent_combat_actions.clear();
        self.spellbook.clear();
        self.inventory.clear();
        self.inventory_complete = false;
        self.equipment.clear();
        self.equipment_complete = false;
        self.actor_actions.clear();
        self.vitality_events.clear();
        self.messages.clear();
        self.revision = revision;

        Change::Projected(revision)
    }

    /// Records closure of the active game transport below the plaintext layer.
    #[instrument(
        name = "viperzoo::world::observe_transport_close",
        skip(self),
        ret(level = "debug")
    )]
    pub fn observe_transport_close(&mut self) -> Change {
        let revision = self.revision.next();

        self.connection = session::Connection::TransportClosed;
        self.revision = revision;

        Change::Projected(revision)
    }
}

fn push_bounded<T>(values: &mut VecDeque<T>, value: T, capacity: usize) {
    if values.len() == capacity {
        let _ = values.pop_front();
    }

    values.push_back(value);
}

#[cfg(test)]
mod tests {
    use viperzoo_adapter_api::inventory::{
        Item as ClientInventoryItem, Snapshot as ClientInventorySnapshot, Source as InventorySource,
    };
    use viperzoo_protocol::{decode, direction::Flow, primitive::Position};

    use super::*;

    fn apply_hex(world: &mut World, value: &str) {
        let bytes = hex::decode(value).expect("test fixture contains valid hex");
        let packet = decode(Flow::Clientbound, &bytes).expect("test packet is structurally valid");
        let _ = world.apply(&packet);
    }

    fn apply_serverbound_hex(world: &mut World, value: &str) {
        let bytes = hex::decode(value).expect("test fixture contains valid hex");
        let packet = decode(Flow::Serverbound, &bytes).expect("test packet is structurally valid");
        let _ = world.apply(&packet);
    }

    #[test]
    fn projection_merges_map_position_resources_and_entities() {
        let mut world = World::new();

        apply_hex(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply_hex(&mut world, "04000300010003000100410000");
        apply_hex(
            &mut world,
            "087900000200030000009700000062040403030400006208a016000000001b000000970000006200000259000000000000000000000001000063bd0000012b62000066276f00",
        );
        apply_hex(
            &mut world,
            "0600000b000e0102003b0000038a002e000100000b030b00",
        );
        apply_hex(&mut world, "070001000b00190500008ac780190501000016a5a600");

        let snapshot = world.snapshot();
        let context = snapshot.map().context().expect("map context is observed");
        let resources = snapshot.player().resources();

        assert_eq!(snapshot.map().epoch().value(), 1);
        assert_eq!(snapshot.map().epoch().origin(), map::Origin::Established);
        assert_eq!(context.title(), Some("Welcome"));
        assert_eq!(
            snapshot.player().location().position(),
            Some(Position::new(3, 1))
        );
        assert_eq!(resources.vita().current().value(), Some(&151));
        assert_eq!(resources.vita().maximum().value(), Some(&151));
        assert_eq!(resources.mana().current().value(), Some(&98));
        assert_eq!(snapshot.player().inventory_capacity().value(), Some(&27));
        assert_eq!(snapshot.player().economy().experience().value(), Some(&601));
        assert_eq!(snapshot.player().economy().money().value(), Some(&0));
        assert_eq!(snapshot.map().tiles().len(), 2);
        assert_eq!(snapshot.map().blocked_tile_count(), 1);
        assert_eq!(snapshot.entities().len(), 1);
        assert_eq!(
            snapshot.entities()[0].appearance().occupancy(),
            viperzoo_protocol::entity::Occupancy::Blocking
        );
    }

    #[test]
    fn partial_resource_update_preserves_known_maxima() {
        let mut world = World::new();

        apply_hex(
            &mut world,
            "087900000200030000009700000062040403030400006208a016000000001b000000970000006200000259000000000000000000000001000063bd0000012b62000066276f00",
        );
        apply_hex(&mut world, "08280000006400000032000000000000000000000000");

        let snapshot = world.snapshot();
        let resources = snapshot.player().resources();

        assert_eq!(resources.vita().current().value(), Some(&100));
        assert_eq!(resources.vita().maximum().value(), Some(&151));
        assert_eq!(resources.mana().current().value(), Some(&50));
        assert_eq!(resources.mana().maximum().value(), Some(&98));
    }

    #[test]
    fn changed_map_identity_clears_only_map_scoped_state() {
        let mut world = World::new();

        apply_hex(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply_hex(&mut world, "04000300010003000100410000");
        apply_hex(
            &mut world,
            "0600000b000e0102003b0000038a002e000100000b030b00",
        );
        apply_hex(&mut world, "070001000b00190500008ac780190501000016a5a600");
        apply_hex(
            &mut world,
            "15014c001e000e04000d537072696e672054617665726e00840002020000",
        );

        let snapshot = world.snapshot();

        assert_eq!(snapshot.map().epoch().value(), 2);
        assert_eq!(snapshot.map().epoch().origin(), map::Origin::Crossed);
        assert!(snapshot.map().tiles().is_empty());
        assert!(snapshot.entities().is_empty());
        assert!(snapshot.player().location().position().is_none());
    }

    /// A warm attachment learning its own identity and a genuine crossing both
    /// advance the epoch counter. Map-scoped work recovers differently from
    /// each, so the projection must keep them distinguishable.
    #[test]
    fn first_identity_and_later_crossing_are_distinguishable() {
        let mut warm = World::new();

        assert_eq!(
            warm.snapshot().map().epoch().origin(),
            map::Origin::Attachment
        );

        // Tiles before any context: provisional coverage.
        apply_hex(
            &mut warm,
            "0600000b000e0102003b0000038a002e000100000b030b00",
        );
        assert_eq!(
            warm.snapshot().map().epoch().origin(),
            map::Origin::Attachment
        );

        // The first `0x15` proves which map that coverage belonged to.
        apply_hex(
            &mut warm,
            "1512670011001105000757656c636f6d6500e80002020200",
        );

        let identified = warm.snapshot().map().epoch();

        assert_eq!(identified.value(), 1);
        assert_eq!(identified.origin(), map::Origin::Established);

        // A second, different identity is the player actually moving.
        apply_hex(
            &mut warm,
            "15014c001e000e04000d537072696e672054617665726e00840002020000",
        );

        let crossed = warm.snapshot().map().epoch();

        assert_eq!(crossed.value(), 2);
        assert_eq!(crossed.origin(), map::Origin::Crossed);
        assert_ne!(identified.origin(), crossed.origin());
    }

    #[test]
    fn movement_and_removal_share_stable_identity() {
        let mut world = World::new();

        apply_hex(&mut world, "070001000b00190500008ac780190501000016a5a600");
        apply_hex(&mut world, "0c00008ac7000b0019000000000000");

        let entity_id = viperzoo_protocol::primitive::EntityId::new(0x8ac7);
        assert_eq!(
            world
                .snapshot()
                .entity(entity_id)
                .expect("movement retains the entity")
                .position(),
            Position::new(11, 24)
        );

        apply_hex(&mut world, "0e00008ac700000000");
        assert!(world.snapshot().entity(entity_id).is_none());
    }

    #[test]
    fn visibility_control_does_not_erase_preceding_appearance_batch() {
        let mut world = World::new();

        apply_hex(&mut world, "070001000b00190500008ac780190501000016a5a600");
        apply_hex(&mut world, "580031002f00");

        assert_eq!(world.snapshot().entities().len(), 1);
    }

    #[test]
    fn snapshots_are_deterministic_for_the_same_observation_order() {
        let observations = [
            "1512670011001105000757656c636f6d6500e80002020200",
            "04000300010003000100410000",
            "0600000b000e0102003b0000038a002e000100000b030b00",
            "070001000b00190500008ac780190501000016a5a600",
        ];
        let mut first = World::new();
        let mut second = World::new();

        for observation in observations {
            apply_hex(&mut first, observation);
            apply_hex(&mut second, observation);
        }

        let first = serde_json::to_string(&first.snapshot()).expect("snapshot serializes");
        let second = serde_json::to_string(&second.snapshot()).expect("snapshot serializes");

        assert_eq!(first, second);
    }

    #[test]
    fn client_actions_are_retained_as_typed_recent_events() {
        let mut world = World::new();
        let packet = decode(
            Flow::Serverbound,
            &hex::decode("130000").expect("valid hex"),
        )
        .expect("captured attack is structurally valid");

        let _ = world.apply(&packet);

        assert!(matches!(
            world.snapshot().recent_actions()[0].action(),
            action::Action::Attack
        ));
        assert!(matches!(
            world.snapshot().recent_combat_actions()[0].action(),
            action::Action::Attack
        ));
    }

    #[test]
    fn movement_and_obstruction_project_explicit_client_local_position() {
        let mut world = World::new();

        apply_hex(&mut world, "04000300010003000100410000");
        let movement = decode(
            Flow::Serverbound,
            &hex::decode("32019750000300010000").expect("valid movement hex"),
        )
        .expect("movement is structurally valid");
        let _ = world.apply(&movement);

        assert_eq!(
            world.snapshot().player().location().position(),
            Some(Position::new(4, 1))
        );
        assert_eq!(
            world.snapshot().player().facing().value(),
            Some(&viperzoo_protocol::direction::Direction::Right)
        );

        let obstruction = decode(
            Flow::Serverbound,
            &hex::decode("69000400010000").expect("valid obstruction hex"),
        )
        .expect("obstruction is structurally valid");
        let _ = world.apply(&obstruction);

        assert_eq!(
            world.snapshot().player().location().position(),
            Some(Position::new(4, 1))
        );
        assert!(matches!(
            world.snapshot().player().location(),
            player::Location::ClientReported { .. }
        ));
        assert_eq!(
            world.snapshot().player().facing().value(),
            Some(&viperzoo_protocol::direction::Direction::Up)
        );
    }

    #[test]
    fn session_boundary_clears_session_scoped_projection() {
        let mut world = World::new();

        apply_hex(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply_hex(&mut world, "04000300010003000100410000");
        let _ = world.begin_session();
        let snapshot = world.snapshot();

        assert!(snapshot.map().context().is_none());
        assert!(snapshot.player().location().position().is_none());
        assert_eq!(snapshot.processed_packet_count(), 0);
    }

    #[test]
    fn first_map_identity_invalidates_warm_scoped_state() {
        let mut world = World::new();

        apply_hex(&mut world, "040015000c0015000c00410000");
        apply_hex(
            &mut world,
            "0600000b000e0102003b0000038a002e000100000b030b00",
        );

        let warm = world.snapshot();
        assert!(warm.map().context().is_none());
        assert_eq!(
            warm.player().location().position(),
            Some(Position::new(21, 12))
        );
        assert!(!warm.map().tiles().is_empty());

        apply_hex(&mut world, "15014a0092009c040004427579610206005eb40000");

        let identified = world.snapshot();
        assert_eq!(
            identified
                .map()
                .context()
                .expect("Buya context")
                .id()
                .value(),
            0x014a
        );
        assert!(identified.player().location().position().is_none());
        assert!(identified.map().tiles().is_empty());

        apply_hex(&mut world, "04002e0069002e006900410000");

        assert_eq!(
            world.snapshot().player().location().position(),
            Some(Position::new(46, 105))
        );
    }

    #[test]
    fn spell_inventory_and_combat_are_script_facing_state() {
        let mut world = World::new();

        apply_hex(&mut world, "17010506536f6f746865024e4f21647900");
        apply_hex(
            &mut world,
            "0f01c0d9000f526162626974206d656174202833290b526162626974206d65617400000003010000000000000096863600",
        );
        apply_hex(&mut world, "130001f55f0064ffffffce01031900");
        apply_hex(&mut world, "1a0001f55f06001e0061737400");
        apply_hex(
            &mut world,
            "0a030010596f75206361737420536f6f7468652e02020200",
        );
        let snapshot = world.snapshot();

        assert_eq!(snapshot.spellbook()[0].name(), "Soothe");
        assert_eq!(snapshot.inventory()[0].amount(), 3);
        assert_eq!(snapshot.vitality_events()[0].amount(), -50);
        assert_eq!(snapshot.actor_actions()[0].kind(), 6);
        assert_eq!(snapshot.messages()[0].text(), "You cast Soothe.");
    }

    #[test]
    fn tree_vitality_is_projected_until_authoritative_removal() {
        let mut world = World::new();

        apply_hex(&mut world, "070001002d00350600017f7d82f92801000000");
        apply_hex(&mut world, "1300017f7d22020000000b01010100");

        let tree = world
            .snapshot()
            .entity(viperzoo_protocol::primitive::EntityId::new(0x0001_7f7d))
            .cloned()
            .expect("tree remains visible before removal");
        assert_eq!(tree.vitality().map(entity::Vitality::percent), Some(2));

        apply_hex(&mut world, "0e00017f7d6f752000");
        assert!(
            world
                .snapshot()
                .entity(viperzoo_protocol::primitive::EntityId::new(0x0001_7f7d))
                .is_none(),
            "the removal packet, not a swing count, completes the lifecycle"
        );
    }

    #[test]
    fn equipping_replacement_axe_reconciles_inventory_and_equipment() {
        let mut world = World::new();

        apply_hex(
            &mut world,
            "0f0ac2850003417865034178650000000100000f424000000002492000",
        );
        apply_serverbound_hex(&mut world, "1c0a000702be00");
        apply_hex(&mut world, "3701c285000341786503417865000f42400000746f2000");
        apply_hex(&mut world, "100a0c000b06000000");

        let snapshot = world.snapshot();
        assert!(snapshot.inventory().is_empty());
        assert_eq!(snapshot.equipment()[0].canonical_name(), "Axe");
        assert!(matches!(
            snapshot.recent_actions()[0].action(),
            action::Action::UseInventory { slot: 10 }
        ));

        apply_hex(&mut world, "380100003700");
        assert!(world.snapshot().equipment().is_empty());
    }

    #[test]
    fn character_profile_distinguishes_complete_empty_equipment_from_unknown() {
        let mut world = World::new();
        assert!(!world.snapshot().equipment_complete());
        assert!(!world.snapshot().inventory_complete());

        let mut profile = String::from("396300000000000000000000680750656173616e74");
        profile.push_str(&"00".repeat(140));
        profile.push_str("01000001001018426f726e20696e204879756c203136312c2053756d6d6572567d7400");
        apply_hex(&mut world, &profile);

        assert!(world.snapshot().equipment_complete());
        assert!(world.snapshot().equipment().is_empty());
        assert!(
            !world.snapshot().inventory_complete(),
            "the equipment profile does not establish carried inventory"
        );

        let _ = world.begin_session();
        assert!(!world.snapshot().equipment_complete());
    }

    #[test]
    fn client_inventory_seed_is_complete_and_later_packets_override_slots() {
        let mut world = World::new();
        let inventory = ClientInventorySnapshot::new(
            27,
            vec![
                ClientInventoryItem::new(10, 0xc285, 0, "Axe", 1).expect("valid memory Axe"),
                ClientInventoryItem::new(11, 0xce99, 0, "Ginko wood", 30)
                    .expect("valid memory Ginko stack"),
            ],
            InventorySource::ClientMemoryBuild752,
        )
        .expect("valid complete inventory");

        let _ = world.seed_inventory(&inventory);
        let seeded = world.snapshot();

        assert!(seeded.inventory_complete());
        assert_eq!(seeded.inventory().len(), 2);
        assert_eq!(seeded.inventory()[1].amount(), 30);
        assert_eq!(
            seeded.inventory()[1].source(),
            inventory::Source::ClientMemoryBuild752
        );

        apply_hex(
            &mut world,
            "0f0ac2850003417865034178650000000100000f424000000002492000",
        );
        let updated = world.snapshot();

        assert!(updated.inventory_complete());
        assert_eq!(
            updated
                .inventory()
                .iter()
                .find(|item| item.slot() == 10)
                .expect("updated Axe remains projected")
                .source(),
            inventory::Source::Protocol
        );

        let _ = world.begin_session();
        assert!(!world.snapshot().inventory_complete());
        assert!(world.snapshot().inventory().is_empty());
    }

    #[test]
    fn pickup_is_a_typed_script_facing_action() {
        let mut world = World::new();

        apply_serverbound_hex(&mut world, "070100");
        assert!(matches!(
            world.snapshot().recent_actions()[0].action(),
            action::Action::Pickup
        ));
    }

    #[test]
    fn refresh_and_disconnect_are_typed_session_actions() {
        let mut world = World::new();

        apply_serverbound_hex(&mut world, "3800");
        apply_serverbound_hex(&mut world, "0b00");
        let snapshot = world.snapshot();

        assert!(matches!(
            snapshot.recent_actions()[0].action(),
            action::Action::Refresh
        ));
        assert!(matches!(
            snapshot.recent_actions()[1].action(),
            action::Action::Disconnect
        ));
        assert_eq!(snapshot.connection(), session::Connection::CloseObserved);

        let _ = world.begin_session();
        assert_eq!(world.snapshot().connection(), session::Connection::Active);
    }

    #[test]
    fn transport_close_is_distinct_from_plaintext_close() {
        let mut world = World::new();

        let change = world.observe_transport_close();

        assert!(change.is_projected());
        assert_eq!(
            world.snapshot().connection(),
            session::Connection::TransportClosed
        );
    }
}
