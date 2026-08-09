//! Describe the complete facts an adapter may give the engine.
//!
//! [`Observation`] is intentionally broader than a packet stream. A fresh
//! attachment establishes a session boundary, a transport close happens below
//! plaintext decoding, and validated client memory can fill a late-attachment
//! gap. Keeping these cases in the same ordered vocabulary lets the reducer
//! apply their precedence and reset rules in one place.

use std::future::Future;

use thiserror::Error;
use viperzoo_protocol::{map, packet::Packet};

use crate::{inventory, resource};

/// One ordered fact from an attached client session.
#[derive(Debug)]
pub enum Observation {
    /// A new client attachment invalidates all session-scoped projection.
    SessionStarted,
    /// The client socket observed an orderly or local transport close.
    TransportClosed,
    /// A decoded plaintext protocol packet was observed.
    Packet(Packet),
    /// A validated late-attachment resource snapshot was read from the client.
    PlayerResources(resource::Resources),
    /// A validated complete carried-inventory snapshot was read from the client.
    PlayerInventory(inventory::Snapshot),
    /// Map identity was read from the client's own model.
    ///
    /// This is the only authoritative answer available to a warm attachment,
    /// which joins after the `0x15` context that names the map has passed. It
    /// establishes identity without describing the map, so it never displaces
    /// a context the server actually sent.
    ClientMap {
        /// Which map, and how large.
        identity: map::Identity,
        /// Display title, when the client's own model exposed one.
        title: Option<String>,
    },
}

impl From<Packet> for Observation {
    fn from(packet: Packet) -> Self {
        Self::Packet(packet)
    }
}

/// A cloneable destination for ordered [`Observation`] values.
///
/// Adapters depend on this narrow capability instead of the runtime that owns
/// the canonical world. Live engines, replay harnesses, and tests can therefore
/// accept the same typed evidence without becoming adapter dependencies.
pub trait Sink: Clone + Send + 'static {
    /// Orders one observation and waits until the receiver has accepted it.
    fn observe(&self, observation: Observation) -> impl Future<Output = Result<(), Error>> + Send;

    /// Orders one observation from a synchronous acquisition boundary.
    ///
    /// # Panics
    ///
    /// Implementations may panic when this method is called from an execution
    /// context that cannot block. Asynchronous adapters should use
    /// [`Sink::observe`] instead.
    fn observe_blocking(&self, observation: Observation) -> Result<(), Error>;
}

/// Failure to deliver evidence to its canonical observation owner.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// The observation owner is no longer accepting evidence.
    #[error("observation sink is closed")]
    Closed,
}
