//! Exclude application-declared tiles from an otherwise valid route.
//!
//! Static collision answers whether a tile can be entered. [`Avoidance`]
//! answers whether this particular route is allowed to enter it. Scripts use
//! it for known portals, guarded areas, or other semantic boundaries whose
//! tiles remain mechanically passable.

use std::collections::BTreeSet;

use viperzoo_protocol::primitive::Position;

/// Immutable route-local tiles that a planner must not enter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Avoidance {
    positions: BTreeSet<Position>,
}

impl Avoidance {
    /// Creates an empty route constraint.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            positions: BTreeSet::new(),
        }
    }

    /// Returns a constraint that excludes `position`.
    #[must_use]
    pub fn with_position(mut self, position: Position) -> Self {
        self.positions.insert(position);
        self
    }

    /// Returns whether a route must avoid `position`.
    #[must_use]
    pub fn avoids(&self, position: Position) -> bool {
        self.positions.contains(&position)
    }

    /// Returns whether this constraint excludes no tiles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}
