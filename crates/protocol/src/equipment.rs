//! Equipped item state sent by the server.

use serde::Serialize;

use crate::primitive::Opaque;

/// One equipped item initialization or replacement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Item {
    slot: u8,
    icon_id: u16,
    icon_color: u8,
    display_name: String,
    canonical_name: String,
    durability: u32,
    opaque_tail: Opaque,
}

impl Item {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        slot: u8,
        icon_id: u16,
        icon_color: u8,
        display_name: String,
        canonical_name: String,
        durability: u32,
        tail: &[u8],
    ) -> Self {
        Self {
            slot,
            icon_id,
            icon_color,
            display_name,
            canonical_name,
            durability,
            opaque_tail: Opaque::from_slice(tail),
        }
    }

    /// Returns the client equipment slot.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }

    /// Returns the display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the canonical item name.
    #[must_use]
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }

    /// Returns the durability reported when the item was equipped.
    #[must_use]
    pub const fn durability(&self) -> u32 {
        self.durability
    }
}

/// One equipped slot removal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Cleared {
    slot: u8,
    opaque_tail: Opaque,
}

impl Cleared {
    pub(crate) fn new(slot: u8, tail: &[u8]) -> Self {
        Self {
            slot,
            opaque_tail: Opaque::from_slice(tail),
        }
    }

    /// Returns the client equipment slot that became empty.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }
}
