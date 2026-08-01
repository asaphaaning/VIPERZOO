//! Direction vocabularies.

use serde::{Deserialize, Serialize};

/// The direction a plaintext body travels relative to the client.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Flow {
    /// A server-to-client body.
    #[serde(alias = "incoming")]
    Clientbound,
    /// A client-to-server body.
    #[serde(alias = "outgoing")]
    Serverbound,
}

impl Flow {
    /// All canonical flow variants.
    pub const VARIANTS: &[Self] = &[Self::Clientbound, Self::Serverbound];
}

/// A cardinal map direction.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Negative vertical movement.
    Up,
    /// Positive horizontal movement.
    Right,
    /// Positive vertical movement.
    Down,
    /// Negative horizontal movement.
    Left,
}

impl Direction {
    /// All canonical direction variants in wire order.
    pub const VARIANTS: &[Self] = &[Self::Up, Self::Right, Self::Down, Self::Left];

    pub(crate) fn from_wire(value: u8) -> Option<Self> {
        Self::VARIANTS.get(usize::from(value)).copied()
    }

    /// Returns the wire representation.
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        match self {
            Self::Up => 0,
            Self::Right => 1,
            Self::Down => 2,
            Self::Left => 3,
        }
    }

    /// Returns the horizontal and vertical coordinate offsets.
    #[must_use]
    pub const fn delta(self) -> (i16, i16) {
        match self {
            Self::Up => (0, -1),
            Self::Right => (1, 0),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
        }
    }
}
