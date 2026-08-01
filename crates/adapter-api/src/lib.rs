//! Define the boundary between acquisition and game semantics.
//!
//! An adapter may read Frida callbacks, replay evidence, or a future network
//! codec. It must translate those details into [`observation::Observation`]
//! before entering the engine. Conversely, scripts ask an adapter to perform
//! an [`action::Action`] through the transport-neutral [`action::Client`] trait.
//!
//! The vocabulary is intentionally closed: it makes session starts, socket
//! closes, packets, and validated memory snapshots explicit rather than
//! allowing individual adapters to smuggle their own state into the world.

pub mod action;
pub mod inventory;
pub mod observation;
pub mod resource;
