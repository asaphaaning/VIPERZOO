//! Query the bounded current server-message history.
//!
//! Messages are transient edges. Use the engine event stream when every
//! occurrence matters; these queries intentionally answer only whether the
//! bounded latest snapshot currently contains matching text.

use viperzoo_protocol::message::Message;

use crate::query::{Evidence, Query, select};

/// Resolves while retained message history contains `fragment`.
pub fn containing(fragment: impl Into<String>) -> impl Query<Output = Evidence<Message>> {
    let fragment = fragment.into();

    select(move |snapshot| {
        snapshot
            .messages()
            .iter()
            .rev()
            .find(|message| message.text().contains(&fragment))
            .cloned()
            .map(|message| Evidence::new(message, snapshot.revision()))
    })
}
