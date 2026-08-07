//! Turn one engine snapshot into the shape a browser can render.
//!
//! This module is deliberately a *narrowing*. The console shows what the
//! engine actually decoded, so every field here traces to an observation:
//! there is no placeholder level, no invented latency, and no sprite atlas.
//! A value the engine has not observed is `null` rather than a plausible
//! default, because a diagnostic window that guesses is worse than one that
//! admits ignorance.
//!
//! ```text
//! world::Snapshot ──► World { map, player, entities, inventory, packets }
//!                              │
//!                              └── assets::Catalog joins object_id ──► blocking
//! ```

use serde::Serialize;
use viperzoo_assets::Catalog;
use viperzoo_world::{map, snapshot::Snapshot};

/// A complete console-facing view of one engine revision.
#[derive(Clone, Debug, Serialize)]
pub struct World {
    /// Monotonic engine revision, shown as the snapshot identifier.
    pub revision: u64,
    /// Whether the client session is still active.
    pub connected: bool,
    /// Decoder progress across every packet this attachment has seen.
    pub packets: Packets,
    /// Session liveness counters.
    pub heartbeat: Heartbeat,
    /// Current map identity and streamed coverage.
    pub map: Map,
    /// Projected character state.
    pub player: Player,
    /// Visible actors, excluding the player.
    pub entities: Vec<Entity>,
    /// Carried items.
    pub inventory: Vec<Item>,
    /// Worn items.
    pub equipment: Vec<Item>,
    /// Recent server text, newest last.
    pub messages: Vec<Message>,
}

/// Decoder coverage measured from real traffic.
///
/// This is the honest version of a coverage meter: the engine counts what it
/// reduced and what it could not, so [`Packets::coverage`] is evidence rather
/// than an estimate.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Packets {
    /// Packets reduced into world state.
    pub processed: u64,
    /// Packets observed but not understood.
    pub unknown: u64,
    /// Understood share of all observed packets, `0.0..=1.0`.
    pub coverage: f32,
}

impl Packets {
    fn from_snapshot(snapshot: &Snapshot) -> Self {
        let processed = snapshot.processed_packet_count();
        let unknown = snapshot.unknown_packet_count();
        let total = processed.saturating_add(unknown);

        Self {
            processed,
            unknown,
            #[expect(
                clippy::cast_precision_loss,
                reason = "a display ratio does not need full u64 precision"
            )]
            coverage: if total == 0 {
                0.0
            } else {
                processed as f32 / total as f32
            },
        }
    }
}

/// Connection liveness as the engine observes it.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Heartbeat {
    /// Server challenges seen.
    pub challenges: u64,
    /// Client responses seen.
    pub pongs: u64,
    /// Responses matched to a challenge.
    pub matched: u64,
}

/// Map identity and the tiles streamed so far.
#[derive(Clone, Debug, Serialize)]
pub struct Map {
    /// Monotonic identity epoch.
    pub epoch: u64,
    /// Why the current epoch began: `attachment`, `established`, or `crossed`.
    pub origin: &'static str,
    /// Server map identifier, absent before the first `0x15`.
    pub id: Option<u16>,
    /// Display title, absent before the first `0x15`.
    pub title: Option<String>,
    /// Decoded width, absent before the first `0x15`.
    pub width: Option<u16>,
    /// Decoded height, absent before the first `0x15`.
    pub height: Option<u16>,
    /// Tiles merged into this epoch's coverage.
    pub tiles: Vec<Tile>,
    /// Tiles the server marks impassable.
    pub blocked: usize,
}

/// One projected map cell.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Tile {
    /// Global column.
    pub x: u16,
    /// Global row.
    pub y: u16,
    /// Ground graphic identifier.
    pub ground: u16,
    /// Server pass value; nonzero is statically blocked.
    pub pass: u16,
    /// Static fixture identifier, zero when absent.
    pub object: u16,
    /// Directional collision mask joined from the client asset catalog.
    ///
    /// `None` when no fixture occupies the tile or the catalog has no record,
    /// which is different from a mask of zero.
    pub collision: Option<u8>,
}

/// One visible actor.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Entity {
    /// Server entity identifier.
    pub id: u32,
    /// Global column.
    pub x: u16,
    /// Global row.
    pub y: u16,
    /// Whether the actor blocks movement into its tile.
    pub blocking: bool,
    /// Whether the actor is a ground item rather than a creature.
    pub floor_item: bool,
}

/// Projected character state, with unobserved values left absent.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Player {
    /// Global column, absent until localization.
    pub x: Option<u16>,
    /// Global row, absent until localization.
    pub y: Option<u16>,
    /// Current vitality.
    pub vita: Option<u32>,
    /// Maximum vitality.
    pub vita_max: Option<u32>,
    /// Current mana.
    pub mana: Option<u32>,
    /// Maximum mana.
    pub mana_max: Option<u32>,
    /// Character level.
    pub level: Option<u8>,
    /// Carried-slot capacity.
    pub capacity: Option<u8>,
    /// Accumulated experience.
    pub experience: Option<u32>,
    /// Carried money.
    pub money: Option<u32>,
}

/// One carried or worn item.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    /// Inventory or equipment slot.
    pub slot: u8,
    /// Name as the client displays it.
    pub name: String,
    /// Stack size; equipment reports one.
    pub amount: u32,
}

/// One line of server text.
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    /// Server-assigned channel or kind byte.
    pub kind: u8,
    /// Rendered text.
    pub text: String,
}

impl World {
    /// Projects one snapshot, joining fixtures against the client catalog.
    ///
    /// `assets` supplies [`Tile::collision`]. Passing `None` simply leaves
    /// that field absent rather than substituting a guess.
    #[must_use]
    pub fn project(snapshot: &Snapshot, assets: Option<&Catalog>) -> Self {
        let map = snapshot.map();
        let context = map.context();
        let player = snapshot.player();
        let position = player.location().position();
        let resources = player.resources();

        Self {
            revision: snapshot.revision().value(),
            connected: matches!(
                snapshot.connection(),
                viperzoo_world::session::Connection::Active
            ),
            packets: Packets::from_snapshot(snapshot),
            heartbeat: Heartbeat {
                challenges: snapshot.heartbeat().challenges_received(),
                pongs: snapshot.heartbeat().pongs_observed(),
                matched: snapshot.heartbeat().matched_pongs(),
            },
            map: Map {
                epoch: map.epoch().value(),
                origin: origin_label(map.epoch().origin()),
                id: context.map(|context| context.id().value()),
                title: context.and_then(|context| context.title().map(str::to_owned)),
                width: context.map(|context| context.dimensions().width()),
                height: context.map(|context| context.dimensions().height()),
                tiles: map
                    .tiles()
                    .iter()
                    .map(|tile| Tile {
                        x: tile.position().x().value(),
                        y: tile.position().y().value(),
                        ground: tile.ground_id(),
                        pass: tile.pass_value(),
                        object: tile.object_id(),
                        collision: collision_of(assets, tile.object_id()),
                    })
                    .collect(),
                blocked: map.blocked_tile_count(),
            },
            player: Player {
                x: position.map(|position| position.x().value()),
                y: position.map(|position| position.y().value()),
                vita: resources.vita().current().value().copied(),
                vita_max: resources.vita().maximum().value().copied(),
                mana: resources.mana().current().value().copied(),
                mana_max: resources.mana().maximum().value().copied(),
                level: player.level().value().copied(),
                capacity: player.inventory_capacity().value().copied(),
                experience: player.economy().experience().value().copied(),
                money: player.economy().money().value().copied(),
            },
            entities: snapshot
                .entities()
                .iter()
                .map(|entity| Entity {
                    id: entity.id().value(),
                    x: entity.position().x().value(),
                    y: entity.position().y().value(),
                    blocking: matches!(
                        entity.appearance().occupancy(),
                        viperzoo_protocol::entity::Occupancy::Blocking
                    ),
                    floor_item: entity.appearance().is_floor_item(),
                })
                .collect(),
            inventory: snapshot
                .inventory()
                .iter()
                .map(|item| Item {
                    slot: item.slot(),
                    name: item.display_name().to_owned(),
                    amount: item.amount(),
                })
                .collect(),
            equipment: snapshot
                .equipment()
                .iter()
                .map(|item| Item {
                    slot: item.slot(),
                    name: item.display_name().to_owned(),
                    amount: 1,
                })
                .collect(),
            messages: snapshot
                .messages()
                .iter()
                .map(|message| Message {
                    kind: message.kind(),
                    text: message.text().to_owned(),
                })
                .collect(),
        }
    }
}

/// Names a map epoch origin for display without leaking `Debug` formatting.
const fn origin_label(origin: map::Origin) -> &'static str {
    match origin {
        map::Origin::Attachment => "attachment",
        map::Origin::Established => "established",
        map::Origin::Crossed => "crossed",
    }
}

/// Joins one map object identifier to its static directional collision mask.
fn collision_of(assets: Option<&Catalog>, object_id: u16) -> Option<u8> {
    if object_id == 0 {
        return None;
    }

    assets
        .and_then(|assets| assets.fixture(object_id))
        .map(|fixture| fixture.collision().bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use viperzoo_world::world::World as Model;

    #[test]
    fn empty_attachment_projects_absent_rather_than_zero() {
        let projected = World::project(&Model::new().snapshot(), None);

        assert_eq!(projected.map.origin, "attachment");
        assert_eq!(projected.map.id, None);
        assert_eq!(projected.player.vita, None);
        assert_eq!(projected.player.level, None);
        assert!(projected.map.tiles.is_empty());
    }

    #[test]
    fn coverage_is_zero_before_any_packet() {
        let projected = World::project(&Model::new().snapshot(), None);

        assert_eq!(projected.packets.processed, 0);
        assert_eq!(projected.packets.unknown, 0);
        assert!(projected.packets.coverage.abs() < f32::EPSILON);
    }
}
