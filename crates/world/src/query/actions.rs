//! Query revisioned player-action history.

use crate::{
    action::{Action, Event},
    query::{Evidence, Query, select},
    revision::Revision,
};

/// Resolves once `expected` appears after `revision` in retained action history.
pub fn observed_after(
    revision: Revision,
    expected: Action,
) -> impl Query<Output = Evidence<Event>> {
    select(move |snapshot| {
        snapshot
            .recent_actions()
            .iter()
            .rev()
            .find(|event| event.revision() > revision && event.action() == &expected)
            .cloned()
            .map(|event| {
                let revision = event.revision();
                Evidence::new(event, revision)
            })
    })
}
