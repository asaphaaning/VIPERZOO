//! Small protocol vocabulary shared by packet families.

use serde::Serialize;

use crate::direction::Direction;

/// A coordinate on one map axis.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Coordinate(u16);

impl Coordinate {
    /// Creates a coordinate from its wire value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the underlying wire value.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Returns the coordinate offset by `amount`, if representable.
    #[must_use]
    pub fn offset(self, amount: i16) -> Option<Self> {
        let value = i32::from(self.0) + i32::from(amount);

        u16::try_from(value).ok().map(Self)
    }
}

/// A tile coordinate in the current map epoch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Position {
    x: Coordinate,
    y: Coordinate,
}

impl Position {
    /// Creates a position from its axis values.
    #[must_use]
    pub const fn new(x: u16, y: u16) -> Self {
        Self {
            x: Coordinate::new(x),
            y: Coordinate::new(y),
        }
    }

    /// Returns the horizontal coordinate.
    #[must_use]
    pub const fn x(self) -> Coordinate {
        self.x
    }

    /// Returns the vertical coordinate.
    #[must_use]
    pub const fn y(self) -> Coordinate {
        self.y
    }

    /// Returns the adjacent position in `direction`, if representable.
    #[must_use]
    pub fn step(self, direction: Direction) -> Option<Self> {
        let (horizontal, vertical) = direction.delta();

        Some(Self {
            x: self.x.offset(horizontal)?,
            y: self.y.offset(vertical)?,
        })
    }
}

/// A stable map identifier from the protocol.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MapId(u16);

impl MapId {
    /// Creates a map identifier from its wire value.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the underlying wire value.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// A stable actor, NPC, or floor-item identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EntityId(u32);

impl EntityId {
    /// Creates an entity identifier from its wire value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the underlying wire value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Opaque bytes retained at a known packet boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Opaque(Vec<u8>);

impl Opaque {
    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    /// Borrows the retained bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Returns whether no opaque bytes were retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// An untouched plaintext logical body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Body(Vec<u8>);

impl Body {
    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    /// Borrows the logical body.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}
