//! Server-authored combat effects.

use serde::Serialize;

use crate::primitive::{EntityId, Opaque};

/// Actor animation/action state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Action {
    actor: EntityId,
    kind: u8,
    reserved: u8,
    duration: u8,
    trailing_reserved: u8,
    opaque_tail: Opaque,
}

impl Action {
    pub(crate) fn new(
        actor: EntityId,
        kind: u8,
        reserved: u8,
        duration: u8,
        trailing_reserved: u8,
        tail: &[u8],
    ) -> Self {
        Self {
            actor,
            kind,
            reserved,
            duration,
            trailing_reserved,
            opaque_tail: Opaque::from_slice(tail),
        }
    }
    /// Returns the acting entity.
    #[must_use]
    pub const fn actor(&self) -> EntityId {
        self.actor
    }
    /// Returns the action type (`1` attack, `6` magic in live evidence).
    #[must_use]
    pub const fn kind(&self) -> u8 {
        self.kind
    }
    /// Returns the animation duration/speed.
    #[must_use]
    pub const fn duration(&self) -> u8 {
        self.duration
    }
}

/// Resulting actor health percentage and signed damage/healing amount.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Vitality {
    actor: EntityId,
    hit_type: u8,
    percent: u8,
    amount: i32,
    opaque_tail: Opaque,
}

impl Vitality {
    pub(crate) fn new(
        actor: EntityId,
        hit_type: u8,
        percent: u8,
        amount: i32,
        tail: &[u8],
    ) -> Self {
        Self {
            actor,
            hit_type,
            percent,
            amount,
            opaque_tail: Opaque::from_slice(tail),
        }
    }
    /// Returns the affected entity.
    #[must_use]
    pub const fn actor(&self) -> EntityId {
        self.actor
    }
    /// Returns the server-authored hit/display classification.
    #[must_use]
    pub const fn hit_type(&self) -> u8 {
        self.hit_type
    }
    /// Returns resulting clamped VITA percentage.
    #[must_use]
    pub const fn percent(&self) -> u8 {
        self.percent
    }
    /// Returns signed amount: positive damage, negative healing.
    #[must_use]
    pub const fn amount(&self) -> i32 {
        self.amount
    }
}
