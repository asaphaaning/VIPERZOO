//! Search the current projection without assuming unseen terrain is safe.
//!
//! The planner combines static streamed tiles, visible entity occupancy,
//! learned directed refusals, and optional fixture collision into one A* graph.
//! It is intentionally a query: it does not submit movement or wait for the
//! map to change.
//!
//! Identified maps can constrain search to decoded dimensions. During warm
//! attachment, the map may have coverage but no identity; search is then
//! limited to that coverage plus a one-tile acquisition halo. A distant target
//! yields [`crate::Plan::Frontier`] rather than an unbounded route through
//! imagined unknown space.
//!
//! A refusal is an ordinary answer here, not a fault. Callers replan after
//! every step and turn [`enum@Error`] into learned edges or an alternative
//! approach tile, so these spans record their error dimension at `debug`.
//! Whether a refusal is worth an operator's attention is the caller's
//! judgement, and only the caller has the context to make it.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BinaryHeap},
};

use thiserror::Error;
use tracing::instrument;
use viperzoo_assets::Catalog;
use viperzoo_protocol::{direction::Direction, entity::Occupancy, primitive::Position};
use viperzoo_world::snapshot::Snapshot;

use crate::{Avoidance, Edge, Knowledge, Plan, Route};

/// Plans from the projected player position to `target`.
///
/// Known blocked map tiles, visible blocking or unresolved entities, known
/// map bounds, and learned directional edges are excluded. Unknown map tiles
/// are traversable so viewport acquisition can extend knowledge during
/// execution on an identified map. An unidentified warm attachment is instead
/// confined to its observed tile rectangle plus a one-tile acquisition halo.
/// A target beyond that rectangle produces a [`Plan::Frontier`] toward it,
/// allowing stepwise viewport expansion without an unbounded A* search.
///
/// # Errors
///
/// Returns [`enum@Error`] when localization is unavailable, the target is
/// outside identified map bounds, or no route exists under current knowledge.
#[instrument(
    name = "viperzoo::navigation::plan",
    skip(snapshot, knowledge),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err(level = "debug"),
    ret(level = "trace")
)]
pub fn plan(snapshot: &Snapshot, knowledge: &Knowledge, target: Position) -> Result<Plan, Error> {
    plan_avoiding(snapshot, knowledge, &Avoidance::new(), target)
}

/// Plans while excluding route-local semantic boundaries.
///
/// Unlike collision, an [`Avoidance`] does not describe the map itself. It is
/// supplied by the caller for a single route, for example to preserve a known
/// portal tile until an explicit portal-crossing transaction selects it.
///
/// # Errors
///
/// Returns the same [`enum@Error`] vocabulary as [`plan`].
#[instrument(
    name = "viperzoo::navigation::plan_avoiding",
    skip(snapshot, knowledge, avoidance),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err(level = "debug"),
    ret(level = "trace")
)]
pub fn plan_avoiding(
    snapshot: &Snapshot,
    knowledge: &Knowledge,
    avoidance: &Avoidance,
    target: Position,
) -> Result<Plan, Error> {
    plan_with(snapshot, knowledge, avoidance, None, target)
}

/// Plans while joining streamed map object identifiers with static client
/// fixture collision masks.
///
/// # Errors
///
/// Returns the same [`enum@Error`] vocabulary as [`plan`].
#[instrument(
    name = "viperzoo::navigation::plan_with_assets",
    skip(snapshot, knowledge, assets),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err(level = "debug"),
    ret(level = "trace")
)]
pub fn plan_with_assets(
    snapshot: &Snapshot,
    knowledge: &Knowledge,
    assets: &Catalog,
    target: Position,
) -> Result<Plan, Error> {
    plan_with_assets_avoiding(snapshot, knowledge, assets, &Avoidance::new(), target)
}

/// Plans with fixture collision while excluding route-local semantic boundaries.
///
/// # Errors
///
/// Returns the same [`enum@Error`] vocabulary as [`plan_with_assets`].
#[instrument(
    name = "viperzoo::navigation::plan_with_assets_avoiding",
    skip(snapshot, knowledge, assets, avoidance),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err(level = "debug"),
    ret(level = "trace")
)]
pub fn plan_with_assets_avoiding(
    snapshot: &Snapshot,
    knowledge: &Knowledge,
    assets: &Catalog,
    avoidance: &Avoidance,
    target: Position,
) -> Result<Plan, Error> {
    plan_with(snapshot, knowledge, avoidance, Some(assets), target)
}

fn plan_with(
    snapshot: &Snapshot,
    knowledge: &Knowledge,
    avoidance: &Avoidance,
    assets: Option<&Catalog>,
    target: Position,
) -> Result<Plan, Error> {
    let origin = snapshot
        .player()
        .location()
        .position()
        .ok_or(Error::PositionUnknown)?;
    let bounds = Bounds::from_snapshot(snapshot, origin, target);

    if origin == target {
        return Ok(Plan::Arrived);
    }

    let goal = bounds.goal(target)?;

    if goal.is_target()
        && (avoidance.avoids(target)
            || blocked_tile(snapshot, target)
            || occupied(snapshot, target))
    {
        return Err(Error::TargetBlocked(target));
    }

    let destination = goal.destination();
    let mut open = BinaryHeap::from([Candidate::new(origin, 0, destination)]);
    let mut best = BTreeMap::from([(origin, 0_u32)]);
    let mut previous = BTreeMap::<Position, (Position, Direction)>::new();

    while let Some(candidate) = open.pop() {
        if candidate.position == destination {
            return reconstruct(origin, goal, &previous);
        }

        if best
            .get(&candidate.position)
            .is_some_and(|known| candidate.cost > *known)
        {
            continue;
        }

        for direction in Direction::VARIANTS.iter().copied() {
            let edge = Edge::new(candidate.position, direction);
            let Some(destination) = edge.destination() else {
                continue;
            };

            if knowledge.is_blocked(edge)
                || !bounds.contains(destination)
                || avoidance.avoids(destination)
                || blocked_tile(snapshot, destination)
                || blocked_fixture(snapshot, assets, destination, direction)
                || occupied(snapshot, destination)
            {
                continue;
            }

            let cost = candidate.cost + 1;

            if best.get(&destination).is_some_and(|known| *known <= cost) {
                continue;
            }

            best.insert(destination, cost);
            previous.insert(destination, (candidate.position, direction));
            open.push(Candidate::new(destination, cost, goal.destination()));
        }
    }

    Err(Error::NoRoute { origin, target })
}

fn blocked_fixture(
    snapshot: &Snapshot,
    assets: Option<&Catalog>,
    destination: Position,
    direction: Direction,
) -> bool {
    assets.is_some_and(|assets| {
        snapshot
            .map()
            .tile(destination)
            .is_some_and(|tile| assets.blocks(tile.object_id(), direction))
    })
}

fn reconstruct(
    origin: Position,
    goal: Goal,
    previous: &BTreeMap<Position, (Position, Direction)>,
) -> Result<Plan, Error> {
    let target = goal.destination();
    let mut cursor = target;
    let mut steps = Vec::new();

    while cursor != origin {
        let Some((parent, direction)) = previous.get(&cursor).copied() else {
            return Err(Error::NoRoute { origin, target });
        };

        steps.push(direction);
        cursor = parent;
    }

    steps.reverse();

    Route::from_steps(steps)
        .map(|route| match goal {
            Goal::Target(_) => Plan::Route(route),
            Goal::Frontier(_) => Plan::Frontier(route),
        })
        .ok_or(Error::NoRoute { origin, target })
}

fn blocked_tile(snapshot: &Snapshot, position: Position) -> bool {
    snapshot
        .map()
        .tile(position)
        .is_some_and(viperzoo_world::map::Tile::blocks_movement)
}

fn occupied(snapshot: &Snapshot, position: Position) -> bool {
    snapshot
        .entities_at(position)
        .any(|entity| entity.appearance().occupancy() != Occupancy::Passable)
}

/// Rectangular area in which a planner can make a safe terrain assumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Bounds {
    minimum: Position,
    maximum: Position,
    kind: BoundsKind,
}

impl Bounds {
    const PROVISIONAL_RADIUS: u16 = 32;
    const ACQUISITION_HALO: u16 = 1;

    fn from_snapshot(snapshot: &Snapshot, origin: Position, target: Position) -> Self {
        if let Some(dimensions) = snapshot
            .map()
            .context()
            .map(viperzoo_world::map::Context::dimensions)
        {
            return Self {
                minimum: Position::new(0, 0),
                maximum: Position::new(
                    dimensions.width().saturating_sub(1),
                    dimensions.height().saturating_sub(1),
                ),
                kind: BoundsKind::Identified,
            };
        }

        Self::observed(snapshot).unwrap_or_else(|| Self::provisional(origin, target))
    }

    fn observed(snapshot: &Snapshot) -> Option<Self> {
        let mut tiles = snapshot.map().tiles().iter();
        let first = tiles.next()?.position();
        let mut minimum = first;
        let mut maximum = first;

        for tile in tiles {
            let position = tile.position();
            minimum = Position::new(
                minimum.x().value().min(position.x().value()),
                minimum.y().value().min(position.y().value()),
            );
            maximum = Position::new(
                maximum.x().value().max(position.x().value()),
                maximum.y().value().max(position.y().value()),
            );
        }

        Some(Self {
            minimum: Position::new(
                minimum.x().value().saturating_sub(Self::ACQUISITION_HALO),
                minimum.y().value().saturating_sub(Self::ACQUISITION_HALO),
            ),
            maximum: Position::new(
                maximum.x().value().saturating_add(Self::ACQUISITION_HALO),
                maximum.y().value().saturating_add(Self::ACQUISITION_HALO),
            ),
            kind: BoundsKind::Observed,
        })
    }

    fn provisional(origin: Position, target: Position) -> Self {
        let minimum = Position::new(
            origin
                .x()
                .value()
                .min(target.x().value())
                .saturating_sub(Self::PROVISIONAL_RADIUS),
            origin
                .y()
                .value()
                .min(target.y().value())
                .saturating_sub(Self::PROVISIONAL_RADIUS),
        );
        let maximum = Position::new(
            origin
                .x()
                .value()
                .max(target.x().value())
                .saturating_add(Self::PROVISIONAL_RADIUS),
            origin
                .y()
                .value()
                .max(target.y().value())
                .saturating_add(Self::PROVISIONAL_RADIUS),
        );

        Self {
            minimum,
            maximum,
            kind: BoundsKind::Provisional,
        }
    }

    const fn contains(self, position: Position) -> bool {
        position.x().value() >= self.minimum.x().value()
            && position.x().value() <= self.maximum.x().value()
            && position.y().value() >= self.minimum.y().value()
            && position.y().value() <= self.maximum.y().value()
    }

    fn goal(self, target: Position) -> Result<Goal, Error> {
        if self.contains(target) {
            return Ok(Goal::Target(target));
        }

        match self.kind {
            BoundsKind::Identified => Err(Error::TargetOutsideMap(target)),
            BoundsKind::Observed => Ok(Goal::Frontier(Position::new(
                target
                    .x()
                    .value()
                    .clamp(self.minimum.x().value(), self.maximum.x().value()),
                target
                    .y()
                    .value()
                    .clamp(self.minimum.y().value(), self.maximum.y().value()),
            ))),
            BoundsKind::Provisional => Ok(Goal::Target(target)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundsKind {
    Identified,
    Observed,
    Provisional,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Goal {
    Target(Position),
    Frontier(Position),
}

impl Goal {
    const fn destination(self) -> Position {
        match self {
            Self::Target(position) | Self::Frontier(position) => position,
        }
    }

    const fn is_target(self) -> bool {
        matches!(self, Self::Target(_))
    }
}

fn distance(left: Position, right: Position) -> u32 {
    u32::from(left.x().value().abs_diff(right.x().value()))
        + u32::from(left.y().value().abs_diff(right.y().value()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Candidate {
    position: Position,
    cost: u32,
    estimate: u32,
}

impl Candidate {
    fn new(position: Position, cost: u32, target: Position) -> Self {
        Self {
            position,
            cost,
            estimate: cost + distance(position, target),
        }
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimate
            .cmp(&self.estimate)
            .then_with(|| other.cost.cmp(&self.cost))
            .then_with(|| other.position.cmp(&self.position))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Route-planning failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// No player position has been projected in this attachment epoch.
    #[error("player position is unknown")]
    PositionUnknown,
    /// The requested target is outside current map dimensions.
    #[error("target {0:?} is outside the current map")]
    TargetOutsideMap(Position),
    /// Static or dynamic occupancy currently blocks the target.
    #[error("target {0:?} is occupied or statically blocked")]
    TargetBlocked(Position),
    /// No path exists under current collision knowledge.
    #[error("no route from {origin:?} to {target:?}")]
    NoRoute {
        /// Projected start tile.
        origin: Position,
        /// Requested destination tile.
        target: Position,
    },
}

#[cfg(test)]
mod tests {
    use viperzoo_assets::{Catalog, Collision, Fixture, Metadata};
    use viperzoo_protocol::{decode, direction::Flow};
    use viperzoo_world::world::World;

    use super::*;

    fn apply(world: &mut World, value: &str) {
        let bytes = hex::decode(value).expect("valid fixture hex");
        let packet = decode(Flow::Clientbound, &bytes).expect("valid fixture packet");
        let _ = world.apply(&packet);
    }

    #[test]
    fn route_avoids_visible_actor() {
        let mut world = World::new();
        apply(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply(&mut world, "04000900050009000500410000");
        apply(&mut world, "070001000b00050500008ac780190501000016a5a600");

        let planned = plan(&world.snapshot(), &Knowledge::new(), Position::new(13, 5))
            .expect("a route around occupancy exists");
        let Plan::Route(route) = planned else {
            panic!("target differs from origin");
        };

        assert!(route.len() > 4);
    }

    #[test]
    fn learned_directional_edge_changes_first_step() {
        let mut world = World::new();
        apply(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply(&mut world, "04000300010003000100410000");
        let edge = Edge::new(Position::new(3, 1), Direction::Right);
        let knowledge = Knowledge::new().with_blocked(edge);
        let Plan::Route(route) = plan(&world.snapshot(), &knowledge, Position::new(5, 1))
            .expect("an alternate route exists")
        else {
            panic!("target differs from origin");
        };

        assert_ne!(route.first(), Some(Direction::Right));
    }

    #[test]
    fn object_830_asset_mask_routes_around_northbound_entry() {
        let mut world = World::new();
        apply(
            &mut world,
            "1512670011001105000757656c636f6d6500e80002020200",
        );
        apply(&mut world, "04000600100006001000410000");
        apply(&mut world, "06000006000f010100000000033e");
        let catalog = Catalog::new(
            [Fixture::new(
                830,
                vec![1878, 1874, 1871],
                Metadata::default(),
                Collision::new(0x01),
            )],
            "fixture-tile.dat".into(),
        );
        let Plan::Route(route) = plan_with_assets(
            &world.snapshot(),
            &Knowledge::new(),
            &catalog,
            Position::new(6, 14),
        )
        .expect("alternate route exists") else {
            panic!("target differs from origin");
        };

        assert_ne!(route.first(), Some(Direction::Up));
    }

    #[test]
    fn warm_attachment_can_plan_before_map_identity_is_observed() {
        let mut world = World::new();
        let movement = hex::decode("32008b50000200170000").expect("valid movement hex");
        let packet = decode(Flow::Serverbound, &movement).expect("valid movement packet");
        let _ = world.apply(&packet);

        let Plan::Route(route) = plan(&world.snapshot(), &Knowledge::new(), Position::new(4, 22))
            .expect("the protocol coordinate domain is a safe provisional bound")
        else {
            panic!("target differs from movement-derived position");
        };

        assert_eq!(route.len(), 2);
        assert_eq!(route.first(), Some(Direction::Right));
    }

    #[test]
    fn warm_attachment_plans_to_the_observed_frontier() {
        let mut world = World::new();
        apply(&mut world, "04000600100006001000410000");
        apply(&mut world, "06000006000f010100000000033e");

        let Plan::Frontier(route) =
            plan(&world.snapshot(), &Knowledge::new(), Position::new(9, 16))
                .expect("a bounded partial route reaches the acquisition frontier")
        else {
            panic!("target remains outside observed map data");
        };

        assert_eq!(route.len(), 1);
        assert_eq!(route.first(), Some(Direction::Right));
    }

    #[test]
    fn route_avoids_a_semantic_portal_tile() {
        let mut world = World::new();
        apply(
            &mut world,
            "1503ea00dc00dc04000a57696c6465726e657373020600b7598d00",
        );
        apply(&mut world, "04007600cf0008000dd91b0000");

        let portal = Position::new(117, 207);
        let Plan::Route(route) = plan_avoiding(
            &world.snapshot(),
            &Knowledge::new(),
            &Avoidance::new().with_position(portal),
            Position::new(110, 178),
        )
        .expect("a route around the portal exists") else {
            panic!("target differs from origin");
        };

        let mut position = Position::new(118, 207);
        for direction in route {
            position = position.step(direction).expect("route remains in bounds");
            assert_ne!(position, portal);
        }
    }
}
