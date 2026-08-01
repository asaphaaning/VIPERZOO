//! Carry a coherent inventory scan across the adapter boundary.
//!
//! Packet updates can reveal individual slots, but a late attachment also needs
//! a complete scan of the client’s carried inventory. [`Snapshot`] validates
//! capacity, one-based slot identity, and uniqueness before it enters the
//! engine, allowing the world to distinguish a partial packet projection from
//! an authoritative complete inventory.

use std::collections::BTreeSet;

use thiserror::Error;

/// One occupied carried-inventory slot read from an attached client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    slot: u8,
    icon_id: u16,
    icon_color: u8,
    name: Box<str>,
    amount: u32,
}

impl Item {
    /// Creates one occupied slot.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the slot, name, or amount cannot represent an
    /// occupied client inventory record.
    pub fn new(
        slot: u8,
        icon_id: u16,
        icon_color: u8,
        name: impl Into<Box<str>>,
        amount: u32,
    ) -> Result<Self, Error> {
        let name = name.into();

        if slot == 0 {
            return Err(Error::ZeroSlot);
        }

        if name.trim().is_empty() {
            return Err(Error::EmptyName { slot });
        }

        if amount == 0 {
            return Err(Error::ZeroAmount { slot });
        }

        Ok(Self {
            slot,
            icon_id,
            icon_color,
            name,
            amount,
        })
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

    /// Returns the stable item name stored in the client model.
    #[must_use]
    pub const fn name(&self) -> &str {
        &self.name
    }

    /// Returns the carried stack amount.
    #[must_use]
    pub const fn amount(&self) -> u32 {
        self.amount
    }
}

/// One coherent scan of every carried-inventory slot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    capacity: u8,
    items: Vec<Item>,
    source: Source,
}

impl Snapshot {
    /// Creates a complete inventory snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the capacity is invalid, an occupied slot lies
    /// outside it, or the same slot appears more than once.
    pub fn new(capacity: u8, items: Vec<Item>, source: Source) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::ZeroCapacity);
        }

        let mut slots = BTreeSet::new();

        for item in &items {
            if item.slot() > capacity {
                return Err(Error::SlotExceedsCapacity {
                    slot: item.slot(),
                    capacity,
                });
            }

            if !slots.insert(item.slot()) {
                return Err(Error::DuplicateSlot(item.slot()));
            }
        }

        Ok(Self {
            capacity,
            items,
            source,
        })
    }

    /// Returns the number of slots covered by the scan.
    #[must_use]
    pub const fn capacity(&self) -> u8 {
        self.capacity
    }

    /// Borrows occupied slots in adapter observation order.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Returns the acquisition source.
    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }

    /// Consumes the snapshot into occupied slots.
    #[must_use]
    pub fn into_items(self) -> Vec<Item> {
        self.items
    }
}

/// The closed vocabulary of validated non-protocol inventory sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// `NexusTK` build 752's persistent inventory model.
    ClientMemoryBuild752,
}

/// Invalid client inventory data.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// A complete inventory scan must cover at least one slot.
    #[error("inventory capacity is zero")]
    ZeroCapacity,
    /// Occupied inventory slots are one-based.
    #[error("inventory slot is zero")]
    ZeroSlot,
    /// An occupied slot must identify an item.
    #[error("inventory slot {slot} has an empty item name")]
    EmptyName {
        /// Invalid one-based slot.
        slot: u8,
    },
    /// An occupied slot must carry at least one item.
    #[error("inventory slot {slot} has a zero amount")]
    ZeroAmount {
        /// Invalid one-based slot.
        slot: u8,
    },
    /// A slot cannot lie beyond the scanned capacity.
    #[error("inventory slot {slot} exceeds capacity {capacity}")]
    SlotExceedsCapacity {
        /// Invalid one-based slot.
        slot: u8,
        /// Scanned capacity.
        capacity: u8,
    },
    /// A complete scan cannot contain duplicate occupied slots.
    #[error("inventory slot {0} was reported more than once")]
    DuplicateSlot(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_snapshot_rejects_duplicate_and_out_of_range_slots() {
        let axe = Item::new(2, 0xc285, 0, "Axe", 1).expect("valid item");

        assert_eq!(
            Snapshot::new(1, vec![axe.clone()], Source::ClientMemoryBuild752),
            Err(Error::SlotExceedsCapacity {
                slot: 2,
                capacity: 1,
            })
        );
        assert_eq!(
            Snapshot::new(2, vec![axe.clone(), axe], Source::ClientMemoryBuild752),
            Err(Error::DuplicateSlot(2))
        );
    }
}
