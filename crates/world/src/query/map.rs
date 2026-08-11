//! Query identified map context, epochs, and static coverage.

use viperzoo_protocol::primitive::Position;

use crate::{
    map::{Context, Epoch, Tile},
    query::{Evidence, Query, select},
};

/// Resolves once the active map has an observed identity.
pub fn identified() -> impl Query<Output = Evidence<Context>> {
    select(|snapshot| {
        snapshot
            .map()
            .context()
            .cloned()
            .map(|context| Evidence::new(context, snapshot.revision()))
    })
}

/// Resolves after the map identity epoch advances beyond `epoch`.
pub fn after(epoch: Epoch) -> impl Query<Output = Evidence<Epoch>> {
    select(move |snapshot| {
        let observed = snapshot.map().epoch();

        (observed.value() > epoch.value()).then(|| Evidence::new(observed, snapshot.revision()))
    })
}

/// Resolves once static map coverage includes `position`.
pub fn tile(position: Position) -> impl Query<Output = Evidence<Tile>> {
    select(move |snapshot| {
        snapshot
            .map()
            .tile(position)
            .map(|tile| Evidence::new(tile, snapshot.revision()))
    })
}
