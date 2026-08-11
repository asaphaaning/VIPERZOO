//! Query authoritative equipped-item state.

use viperzoo_protocol::equipment::Item;

use crate::query::{Evidence, Query, select};

/// Resolves once a complete character profile established every equipment slot.
pub fn complete() -> impl Query<Output = Evidence<Vec<Item>>> {
    select(|snapshot| {
        snapshot
            .equipment_complete()
            .then(|| Evidence::new(snapshot.equipment().to_vec(), snapshot.revision()))
    })
}

/// Resolves once complete equipment contains `canonical_name`.
pub fn item(canonical_name: impl Into<String>) -> impl Query<Output = Evidence<Item>> {
    let canonical_name = canonical_name.into();

    select(move |snapshot| {
        snapshot.equipment_complete().then_some(())?;
        snapshot
            .equipment()
            .iter()
            .find(|item| item.canonical_name() == canonical_name)
            .cloned()
            .map(|item| Evidence::new(item, snapshot.revision()))
    })
}
