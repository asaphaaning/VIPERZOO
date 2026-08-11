//! Query authoritative carried-inventory state.

use crate::{
    inventory::Item,
    query::{Evidence, Query, select},
};

/// Resolves once an authoritative inventory scan has established every slot.
pub fn complete() -> impl Query<Output = Evidence<Vec<Item>>> {
    select(|snapshot| {
        snapshot
            .inventory_complete()
            .then(|| Evidence::new(snapshot.inventory().to_vec(), snapshot.revision()))
    })
}

/// Resolves once a complete inventory contains an item with `canonical_name`.
pub fn item(canonical_name: impl Into<String>) -> impl Query<Output = Evidence<Item>> {
    let canonical_name = canonical_name.into();

    select(move |snapshot| {
        snapshot.inventory_complete().then_some(())?;
        snapshot
            .inventory()
            .iter()
            .find(|item| item.canonical_name() == canonical_name)
            .cloned()
            .map(|item| Evidence::new(item, snapshot.revision()))
    })
}

/// Resolves with the total amount once inventory completeness is known.
pub fn amount(canonical_name: impl Into<String>) -> impl Query<Output = Evidence<u32>> {
    let canonical_name = canonical_name.into();

    select(move |snapshot| {
        snapshot.inventory_complete().then_some(())?;
        let amount = snapshot
            .inventory()
            .iter()
            .filter(|item| item.canonical_name() == canonical_name)
            .map(Item::amount)
            .sum();

        Some(Evidence::new(amount, snapshot.revision()))
    })
}
