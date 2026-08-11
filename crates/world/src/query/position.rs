//! Query canonical player localization.

use viperzoo_protocol::primitive::Position;

use crate::query::{Evidence, Query, select};

/// Resolves once the player has a canonical position.
pub fn known() -> impl Query<Output = Evidence<Position>> {
    select(|snapshot| {
        snapshot
            .player()
            .location()
            .position()
            .map(|position| Evidence::new(position, snapshot.revision()))
    })
}

/// Resolves once the canonical player position equals `expected`.
pub fn at(expected: Position) -> impl Query<Output = Evidence<Position>> {
    select(move |snapshot| {
        (snapshot.player().location().position() == Some(expected))
            .then(|| Evidence::new(expected, snapshot.revision()))
    })
}
