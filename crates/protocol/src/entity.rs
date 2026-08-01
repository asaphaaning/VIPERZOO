//! Visible entity lifecycle observations.

use serde::Serialize;

use crate::direction::Direction;
use crate::primitive::{EntityId, Opaque, Position};

/// The visual record type of an appeared entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A non-blocking floor item.
    FloorItem,
    /// A blocking actor or animal.
    Actor,
    /// A blocking NPC.
    Npc,
    /// A record type whose semantics remain unknown.
    Unknown(u8),
}

impl Kind {
    pub(crate) const fn from_wire(value: u8) -> Self {
        match value {
            0x02 => Self::FloorItem,
            0x05 => Self::Actor,
            0x0C => Self::Npc,
            value => Self::Unknown(value),
        }
    }

    /// Returns static occupancy knowledge implied by the record type.
    #[must_use]
    pub const fn occupancy(self) -> Occupancy {
        match self {
            Self::FloorItem => Occupancy::Passable,
            Self::Actor | Self::Npc => Occupancy::Blocking,
            Self::Unknown(_) => Occupancy::Unknown,
        }
    }
}

/// What an appearance record establishes about movement occupancy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Occupancy {
    /// The record type does not yet establish collision semantics.
    Unknown,
    /// The entity does not block player movement.
    Passable,
    /// The entity blocks player movement.
    Blocking,
}

/// One entity animation descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Animation {
    id: u16,
    duration: u16,
}

impl Animation {
    pub(crate) const fn new(id: u16, duration: u16) -> Self {
        Self { id, duration }
    }

    /// Returns the animation identifier.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.id
    }

    /// Returns the animation duration.
    #[must_use]
    pub const fn duration(self) -> u16 {
        self.duration
    }
}

/// One newly visible entity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Appearance {
    id: EntityId,
    position: Position,
    kind: Kind,
    look_id: u16,
    look_color: u8,
    direction: Direction,
    unknown: u8,
    animations: Vec<Animation>,
}

impl Appearance {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: EntityId,
        position: Position,
        kind: Kind,
        look_id: u16,
        look_color: u8,
        direction: Direction,
        unknown: u8,
        animations: Vec<Animation>,
    ) -> Self {
        Self {
            id,
            position,
            kind,
            look_id,
            look_color,
            direction,
            unknown,
            animations,
        }
    }

    /// Returns the stable runtime identifier.
    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.id
    }

    /// Returns the initial map position.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Returns the typed visual record kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Returns the visual look identifier.
    #[must_use]
    pub const fn look_id(&self) -> u16 {
        self.look_id
    }

    /// Returns the look color.
    #[must_use]
    pub const fn look_color(&self) -> u8 {
        self.look_color
    }

    /// Returns the initial facing direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns the unresolved appearance byte.
    #[must_use]
    pub const fn unknown(&self) -> u8 {
        self.unknown
    }

    /// Borrows the animation descriptors.
    #[must_use]
    pub fn animations(&self) -> &[Animation] {
        &self.animations
    }
}

/// A batch of newly visible entities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Appearances {
    entities: Vec<Appearance>,
    pass_flag: u8,
    opaque_tail: Opaque,
}

impl Appearances {
    pub(crate) fn new(entities: Vec<Appearance>, pass_flag: u8, opaque_tail: Opaque) -> Self {
        Self {
            entities,
            pass_flag,
            opaque_tail,
        }
    }

    /// Borrows the appeared entities.
    #[must_use]
    pub fn entities(&self) -> &[Appearance] {
        &self.entities
    }

    /// Returns the shared pass flag following the records.
    #[must_use]
    pub const fn pass_flag(&self) -> u8 {
        self.pass_flag
    }

    /// Returns retained workspace bytes.
    #[must_use]
    pub const fn opaque_tail(&self) -> &Opaque {
        &self.opaque_tail
    }
}

/// One entity movement from a known pre-step tile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Movement {
    id: EntityId,
    origin: Position,
    direction: Direction,
    opaque_tail: Opaque,
}

impl Movement {
    pub(crate) fn new(
        id: EntityId,
        origin: Position,
        direction: Direction,
        opaque_tail: Opaque,
    ) -> Self {
        Self {
            id,
            origin,
            direction,
            opaque_tail,
        }
    }

    /// Returns the stable runtime identifier.
    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.id
    }

    /// Returns the reported pre-step tile.
    #[must_use]
    pub const fn origin(&self) -> Position {
        self.origin
    }

    /// Returns the movement direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns the derived destination if it fits the coordinate domain.
    #[must_use]
    pub fn destination(&self) -> Option<Position> {
        self.origin.step(self.direction)
    }
}

/// A stable entity removal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Removal {
    id: EntityId,
    opaque_tail: Opaque,
}

impl Removal {
    pub(crate) fn new(id: EntityId, opaque_tail: Opaque) -> Self {
        Self { id, opaque_tail }
    }

    /// Returns the removed entity identifier.
    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.id
    }
}

/// An unresolved visibility control observation.
///
/// Live refresh captures place this packet immediately after a complete
/// [`Appearances`] batch. It therefore does not, by itself, establish that
/// any projected entities became stale.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Control {
    reserved: u8,
    opaque_tail: Opaque,
}

impl Control {
    pub(crate) fn new(reserved: u8, opaque_tail: Opaque) -> Self {
        Self {
            reserved,
            opaque_tail,
        }
    }

    /// Returns the reserved protocol byte.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }
}
