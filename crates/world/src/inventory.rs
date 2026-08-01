//! Script-facing carried-inventory projection.

use serde::Serialize;
use viperzoo_adapter_api::inventory as adapter;
use viperzoo_protocol::inventory as protocol;

/// One occupied carried-inventory slot with an acquisition source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Item {
    slot: u8,
    icon_id: u16,
    icon_color: u8,
    display_name: String,
    canonical_name: String,
    amount: u32,
    durability: Option<u32>,
    source: Source,
}

impl Item {
    pub(crate) fn from_packet(item: &protocol::Item) -> Self {
        Self {
            slot: item.slot(),
            icon_id: item.icon_id(),
            icon_color: item.icon_color(),
            display_name: item.display_name().to_owned(),
            canonical_name: item.canonical_name().to_owned(),
            amount: item.amount(),
            durability: Some(item.durability()),
            source: Source::Protocol,
        }
    }

    pub(crate) fn from_client(item: &adapter::Item) -> Self {
        Self {
            slot: item.slot(),
            icon_id: item.icon_id(),
            icon_color: item.icon_color(),
            display_name: item.name().to_owned(),
            canonical_name: item.name().to_owned(),
            amount: item.amount(),
            durability: None,
            source: Source::ClientMemoryBuild752,
        }
    }

    /// Returns the one-based inventory slot.
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

    /// Returns the stack amount.
    #[must_use]
    pub const fn amount(&self) -> u32 {
        self.amount
    }

    /// Returns durability when the packet stream established it.
    #[must_use]
    pub const fn durability(&self) -> Option<u32> {
        self.durability
    }

    /// Returns the observation that established this slot.
    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }
}

/// The observation that established one projected inventory slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Incoming `0x0F` carried-slot initialization or update.
    Protocol,
    /// `NexusTK` build 752's validated persistent inventory model.
    ClientMemoryBuild752,
}
