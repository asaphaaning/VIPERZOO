//! Project ordered observations into one immutable game world.
//!
//! [`world::World`] is the deterministic domain model beneath the asynchronous
//! engine. It joins packet-derived facts with narrowly validated
//! late-attachment facts, preserves where a value came from in [`knowledge`],
//! and advances a revision even when an observation changes only evidence.
//!
//! A [`snapshot::Snapshot`] is a coherent read of that model. Scripts inspect
//! snapshots; only the engine owns mutation. This makes replay, live
//! acquisition, and tests agree about what the same observation sequence means.
//!
//! ```text
//! Ordered observations ──► World ──► Change { Recorded | Projected }
//!                              │
//!                              └──► Snapshot: one revision, many related facts
//! ```

pub mod action;
pub mod entity;
pub mod inventory;
pub mod knowledge;
pub mod map;
pub mod player;
pub mod revision;
pub mod session;
pub mod snapshot;
pub mod world;
