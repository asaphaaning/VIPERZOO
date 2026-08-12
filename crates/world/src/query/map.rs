//! Query identified map context, epochs, and static coverage.

use viperzoo_protocol::primitive::{MapId, Position};

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

/// Resolves once the active map has an observed [`MapId`].
///
/// This is the identity-shaped counterpart to [`identified`]. It avoids
/// forcing applications that only need to name the active map to project a
/// complete [`Context`] or unwrap its wire representation.
pub fn id() -> impl Query<Output = Evidence<MapId>> {
    select(|snapshot| {
        snapshot
            .map()
            .id()
            .map(|id| Evidence::new(id, snapshot.revision()))
    })
}

/// Resolves while `expected` is the active map identity.
///
/// A different identified map remains pending rather than resolving to
/// `false`, so this query composes directly with engine waits and semantic
/// action confirmation.
pub fn is(expected: MapId) -> impl Query<Output = Evidence<MapId>> {
    select(move |snapshot| {
        (snapshot.map().id() == Some(expected))
            .then(|| Evidence::new(expected, snapshot.revision()))
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

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{
        map::{Dimensions, Identity},
        primitive::MapId,
    };

    use crate::{
        query::{Query, State},
        world::World,
    };

    use super::{id, is};

    const GROVE: MapId = MapId::new(0x0482);
    const WILDERNESS: MapId = MapId::new(0x03ea);

    #[test]
    fn id_preserves_the_revision_that_established_identity() {
        let mut world = World::new();
        let identity = Identity::new(GROVE, Dimensions::new(60, 60).unwrap());
        let change = world.seed_map_identity(identity, Some("Golden Grove".to_owned()));
        let snapshot = world.snapshot();

        let State::Ready(evidence) = id().evaluate(&snapshot) else {
            panic!("seeded map identity should resolve the map query");
        };

        assert_eq!(*evidence.value(), GROVE);
        assert_eq!(evidence.revision(), change.revision());
    }

    #[test]
    fn is_waits_for_the_requested_map() {
        let mut world = World::new();
        let identity = Identity::new(GROVE, Dimensions::new(60, 60).unwrap());
        world.seed_map_identity(identity, None);
        let snapshot = world.snapshot();

        assert!(matches!(
            is(GROVE).evaluate(&snapshot),
            State::Ready(evidence) if *evidence.value() == GROVE
        ));
        assert_eq!(is(WILDERNESS).evaluate(&snapshot), State::Pending);
    }
}
