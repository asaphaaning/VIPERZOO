//! Inventory slot state sent by the server.

use serde::Serialize;

use crate::primitive::Opaque;

/// One inventory slot removal.
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

    /// Returns the one-based inventory slot that became empty.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }
}

/// One inventory slot initialization or update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Item {
    slot: u8,
    icon_id: u16,
    icon_color: u8,
    display_name: String,
    canonical_name: String,
    amount: u32,
    stack_mode: u8,
    durability: u32,
    protected: u8,
    owner: String,
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
        amount: u32,
        stack_mode: u8,
        durability: u32,
        protected: u8,
        owner: String,
        tail: &[u8],
    ) -> Self {
        Self {
            slot,
            icon_id,
            icon_color,
            display_name,
            canonical_name,
            amount,
            stack_mode,
            durability,
            protected,
            owner,
            opaque_tail: Opaque::from_slice(tail),
        }
    }

    /// Returns the one-based inventory slot.
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }
    /// Returns the client item/icon identifier.
    #[must_use]
    pub const fn icon_id(&self) -> u16 {
        self.icon_id
    }
    /// Returns the client item/icon color.
    #[must_use]
    pub const fn icon_color(&self) -> u8 {
        self.icon_color
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
    /// Returns the stack amount.
    #[must_use]
    pub const fn amount(&self) -> u32 {
        self.amount
    }
    /// Returns remaining durability.
    #[must_use]
    pub const fn durability(&self) -> u32 {
        self.durability
    }
}
