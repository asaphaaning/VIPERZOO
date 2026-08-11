//! Own the canonical world projection.
//!
//! [`Reducer`] is the synchronous, deterministic form: it turns one ordered
//! [`viperzoo_adapter_api::observation::Observation`] into a new world state.
//! It is useful for replay and tests. [`channel`] adds the live ownership
//! boundary: one [`Owner`] reduces every observation in order, cloneable
//! [`Ingress`] values submit work, and cloneable [`World`] values watch
//! immutable [`viperzoo_world::snapshot::Snapshot`]s and subscribe to ordered
//! [`viperzoo_world::event::Event`] edges. [`Channel::spawn`] schedules that
//! owner and returns a named [`Running`] product when an opinionated
//! Tokio-owned lifecycle is desired.
//!
//! This separation keeps concurrency out of the domain model. Adapters never
//! mutate a [`viperzoo_world::world::World`] directly, and readers cannot
//! observe a partially applied packet.
//!
//! ```text
//! Ingress ── Observation ──► Owner ──► Reducer ──► Arc<Snapshot> ──► World.latest
//!                              │            │
//!                              │            └── Event ──► World.subscribe
//!                              └── Receipt ──► Ingress
//! ```

mod reducer;
mod runtime;

pub use reducer::Reducer;
pub use runtime::{
    Channel, Config, Cursor, Error, EventError, Events, Ingress, Owner, Receipt, Running,
    SpawnError, Subscription, SubscriptionError, Task, TaskError, Wait, WaitError, World, channel,
};
