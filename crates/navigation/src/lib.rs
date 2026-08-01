//! Plan deterministic movement from immutable world knowledge.
//!
//! [`plan`] is a pure A* query over a [`viperzoo_world::snapshot::Snapshot`]
//! and per-epoch traversal [`Knowledge`]. It combines streamed tile coverage,
//! visible occupancy, optional client fixture collision, and learned refusal
//! edges—but has no timers, Frida handles, or input policy.
//!
//! A [`Plan::Frontier`] is important for warm attachment: it reaches the edge
//! of observed coverage when the target remains outside it. A controller can
//! execute one safe step, receive more map data, and plan again without
//! pretending unknown coordinates are globally traversable.

mod edge;
mod planner;
mod route;

pub use edge::{Edge, Knowledge};
pub use planner::{Error, plan, plan_with_assets};
pub use route::{Plan, Route};
