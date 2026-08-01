//! Own the canonical world projection.
//!
//! [`Reducer`] is the synchronous, deterministic form: it turns one ordered
//! [`viperzoo_adapter_api::observation::Observation`] into a new world state.
//! It is useful for replay and tests. [`channel`] adds the live ownership
//! boundary: one [`Task`] reduces every observation in order, while cloneable
//! [`Handle`] values submit work and watch immutable [`viperzoo_world::snapshot::Snapshot`]s.
//!
//! This separation keeps concurrency out of the domain model. Adapters never
//! mutate a [`viperzoo_world::world::World`] directly, and readers cannot
//! observe a partially applied packet.
//!
//! ```text
//! Adapter threads ── Observation ──► Task ──► Reducer ──► Arc<Snapshot>
//!                                      │                       │
//!                                      └── Receipt ◄── Handle ──┘
//! ```

mod reducer;
mod runtime;

pub use reducer::Reducer;
pub use runtime::{Config, Error, Handle, Receipt, Task, channel};
