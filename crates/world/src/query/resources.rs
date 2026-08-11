//! Query complete player resource knowledge.

use crate::{
    player::Resources,
    query::{Evidence, Query, select},
};

/// Resolves once current and maximum VITA and mana are all known.
pub fn known() -> impl Query<Output = Evidence<Resources>> {
    select(|snapshot| {
        let resources = snapshot.player().resources();
        let known = resources.vita().current().value().is_some()
            && resources.vita().maximum().value().is_some()
            && resources.mana().current().value().is_some()
            && resources.mana().maximum().value().is_some();

        known.then(|| Evidence::new(resources.clone(), snapshot.revision()))
    })
}
