//! Retain learned movement refusals separately from map collision.
//!
//! [`Edge`] is directed because entering the same tile from two directions can
//! have different outcomes. [`Knowledge`] records only runtime evidence from
//! the current map epoch, such as a silent client refusal. It is deliberately
//! separate from decoded terrain and fixture collision: policy may revisit or
//! clear learned evidence, but it must never weaken authoritative static data.

use std::collections::BTreeSet;

use viperzoo_protocol::{direction::Direction, primitive::Position};

/// One directed movement edge.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Edge {
    origin: Position,
    direction: Direction,
}

impl Edge {
    /// Creates an edge from a tile and cardinal direction.
    #[must_use]
    pub const fn new(origin: Position, direction: Direction) -> Self {
        Self { origin, direction }
    }

    /// Returns the origin tile.
    #[must_use]
    pub const fn origin(self) -> Position {
        self.origin
    }

    /// Returns the attempted direction.
    #[must_use]
    pub const fn direction(self) -> Direction {
        self.direction
    }

    /// Returns the destination when it is representable.
    #[must_use]
    pub fn destination(self) -> Option<Position> {
        self.origin.step(self.direction)
    }
}

/// Collision evidence learned while controlling the current map epoch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Knowledge {
    blocked: BTreeSet<Edge>,
}

impl Knowledge {
    /// Creates empty traversal knowledge.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            blocked: BTreeSet::new(),
        }
    }

    /// Returns a value with `edge` marked blocked.
    #[must_use]
    pub fn with_blocked(mut self, edge: Edge) -> Self {
        self.blocked.insert(edge);
        self
    }

    /// Records a blocked edge, returning whether it was new evidence.
    pub fn block(&mut self, edge: Edge) -> bool {
        self.blocked.insert(edge)
    }

    /// Returns whether the edge has been observed as blocked.
    #[must_use]
    pub fn is_blocked(&self, edge: Edge) -> bool {
        self.blocked.contains(&edge)
    }

    /// Returns whether no runtime refusal edges are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }

    /// Clears knowledge invalidated by a map-epoch change.
    pub fn clear(&mut self) {
        self.blocked.clear();
    }
}
