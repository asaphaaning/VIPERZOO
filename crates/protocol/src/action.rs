//! High-confidence client action requests.

use serde::Serialize;

use crate::{
    direction::Direction,
    primitive::{EntityId, MapId, Opaque, Position},
};

/// One client-authored speech line.
///
/// The current client encodes [`text`](Self::text) as ASCII followed by a NUL
/// terminator. The preceding length excludes that terminator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Speech {
    channel: u8,
    text: Box<str>,
}

impl Speech {
    pub(crate) fn new(channel: u8, text: Box<str>) -> Self {
        Self { channel, text }
    }

    /// Returns the client-selected speech channel.
    #[must_use]
    pub const fn channel(&self) -> u8 {
        self.channel
    }

    /// Returns the exact human-readable speech text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// One request to interact with a visible entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Interact {
    mode: u8,
    entity: EntityId,
    reserved: u8,
}

impl Interact {
    pub(crate) const fn new(mode: u8, entity: EntityId, reserved: u8) -> Self {
        Self {
            mode,
            entity,
            reserved,
        }
    }

    /// Returns the target actor or NPC.
    #[must_use]
    pub const fn entity(self) -> EntityId {
        self.entity
    }
}

/// One structured response in an NPC dialog.
///
/// `tail` preserves command-specific bytes which are not yet part of the
/// shared dialog grammar. For example, the observed shop-confirmation command
/// carries a quantity suffix after its item-name argument.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Dialog {
    mode: u8,
    entity: EntityId,
    command: u8,
    argument: Option<Box<str>>,
    tail: Opaque,
}

impl Dialog {
    pub(crate) const fn new(
        mode: u8,
        entity: EntityId,
        command: u8,
        argument: Option<Box<str>>,
        tail: Opaque,
    ) -> Self {
        Self {
            mode,
            entity,
            command,
            argument,
            tail,
        }
    }

    /// Returns the NPC receiving the response.
    #[must_use]
    pub const fn entity(&self) -> EntityId {
        self.entity
    }

    /// Returns the server-authored dialog command token.
    #[must_use]
    pub const fn command(&self) -> u8 {
        self.command
    }

    /// Returns the optional ASCII dialog argument.
    #[must_use]
    pub fn argument(&self) -> Option<&str> {
        self.argument.as_deref()
    }

    /// Returns command-specific trailing bytes retained without inference.
    #[must_use]
    pub fn tail(&self) -> &Opaque {
        &self.tail
    }
}

/// One destination selection from a server-provided travel menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TravelSelection {
    map: MapId,
    position: Position,
    reserved: u8,
}

impl TravelSelection {
    pub(crate) const fn new(map: MapId, position: Position, reserved: u8) -> Self {
        Self {
            map,
            position,
            reserved,
        }
    }

    /// Returns the selected destination map identifier.
    #[must_use]
    pub const fn map(self) -> MapId {
        self.map
    }

    /// Returns the destination entry's map coordinate.
    #[must_use]
    pub const fn position(self) -> Position {
        self.position
    }
}

/// One request to pick up an item from the player's tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Pickup {
    mode: u8,
    reserved: u8,
}

/// One client request to redraw and resynchronize the visible map.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Refresh {
    reserved: u8,
}

impl Refresh {
    pub(crate) const fn new(reserved: u8) -> Self {
        Self { reserved }
    }

    /// Returns the currently zero-valued trailing field.
    #[must_use]
    pub const fn reserved(self) -> u8 {
        self.reserved
    }
}

/// One clean client-initiated session close request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Disconnect {
    reason: u8,
}

impl Disconnect {
    pub(crate) const fn new(reason: u8) -> Self {
        Self { reason }
    }

    /// Returns the client-provided close reason (`0` in current live evidence).
    #[must_use]
    pub const fn reason(self) -> u8 {
        self.reason
    }
}

impl Pickup {
    pub(crate) const fn new(mode: u8, reserved: u8) -> Self {
        Self { mode, reserved }
    }

    /// Returns the observed pickup mode (`1` in current live evidence).
    #[must_use]
    pub const fn mode(self) -> u8 {
        self.mode
    }
}

/// One request to activate an item from an inventory slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct UseInventory {
    slot: u8,
    actor: EntityId,
    reserved: u8,
}

impl UseInventory {
    pub(crate) const fn new(slot: u8, actor: EntityId, reserved: u8) -> Self {
        Self {
            slot,
            actor,
            reserved,
        }
    }

    /// Returns the one-based inventory slot.
    #[must_use]
    pub const fn slot(self) -> u8 {
        self.slot
    }

    /// Returns the player entity encoded by the client.
    #[must_use]
    pub const fn actor(self) -> EntityId {
        self.actor
    }
}

/// One client movement request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum Movement {
    /// Movement carrying the newly exposed viewport-region request.
    WithMapRegion {
        /// Requested direction.
        direction: Direction,
        /// Wrapping client movement counter.
        last_walk: u8,
        /// Client movement speed.
        speed: u8,
        /// Position before applying the step.
        origin: Position,
        /// Newly exposed map-region origin.
        region_origin: Position,
        /// Requested region width.
        region_width: u8,
        /// Requested region height.
        region_height: u8,
        /// Client cache checksum for the requested region.
        checksum: u16,
        /// Plaintext-hook workspace tail.
        opaque_tail: Opaque,
    },
    /// Compact movement without a viewport-region request.
    Compact {
        /// Requested direction.
        direction: Direction,
        /// Wrapping client movement counter.
        last_walk: u8,
        /// Client movement speed.
        speed: u8,
        /// Position before applying the step.
        origin: Position,
        /// Plaintext-hook workspace tail.
        opaque_tail: Opaque,
    },
}

impl Movement {
    /// Returns the requested direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        match self {
            Self::WithMapRegion { direction, .. } | Self::Compact { direction, .. } => *direction,
        }
    }

    /// Returns the position before applying the requested step.
    #[must_use]
    pub const fn origin(&self) -> Position {
        match self {
            Self::WithMapRegion { origin, .. } | Self::Compact { origin, .. } => *origin,
        }
    }

    /// Returns the wrapping client movement counter.
    #[must_use]
    pub const fn last_walk(&self) -> u8 {
        match self {
            Self::WithMapRegion { last_walk, .. } | Self::Compact { last_walk, .. } => *last_walk,
        }
    }
}

/// A client-side report that a movement edge was obstructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Obstruction {
    origin: Position,
    direction: Direction,
    reserved: u8,
}

impl Obstruction {
    pub(crate) const fn new(origin: Position, direction: Direction, reserved: u8) -> Self {
        Self {
            origin,
            direction,
            reserved,
        }
    }

    /// Returns the unchanged coordinate reported by the client.
    #[must_use]
    pub const fn origin(self) -> Position {
        self.origin
    }

    /// Returns the direction whose adjacent edge was rejected.
    #[must_use]
    pub const fn direction(self) -> Direction {
        self.direction
    }
}

/// A direction-only facing request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Facing {
    direction: Direction,
    reserved: u8,
}

impl Facing {
    pub(crate) const fn new(direction: Direction, reserved: u8) -> Self {
        Self {
            direction,
            reserved,
        }
    }

    /// Returns the requested facing direction.
    #[must_use]
    pub const fn direction(self) -> Direction {
        self.direction
    }
}

/// One ordinary attack request in the current facing direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Attack {
    parameters: u16,
}

impl Attack {
    pub(crate) const fn new(parameters: u16) -> Self {
        Self { parameters }
    }

    /// Returns the currently fixed little-endian action parameters.
    #[must_use]
    pub const fn parameters(self) -> u16 {
        self.parameters
    }
}

/// One spell invocation request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Cast {
    slot: u8,
    payload: Opaque,
}

impl Cast {
    pub(crate) const fn new(slot: u8, payload: Opaque) -> Self {
        Self { slot, payload }
    }

    /// Returns the one-based spellbook slot.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }

    /// Returns target, question, or workspace bytes after the slot.
    #[must_use]
    pub const fn payload(&self) -> &Opaque {
        &self.payload
    }
}
