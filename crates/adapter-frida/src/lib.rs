//! Attach Frida to a live `NexusTK` client without leaking Frida into the domain.
//!
//! Frida's object graph is not `Send`, while the engine and scripts are
//! asynchronous. [`Driver`] therefore owns every Frida value on one dedicated
//! operating-system thread. [`Control`] sends typed client intents across that
//! boundary, and callback handling forwards validated observations through the
//! adapter API's narrow sink capability.
//!
//! The bundled JavaScript agent installs hooks only. Packet decoding, world
//! ownership, and action policy remain Rust responsibilities.
//!
//! ```text
//! Script ── Action ──► Control ──► dedicated Frida thread ──► client
//!                                      │
//! client callback ── plaintext body ──┴──► Observation ──► engine
//! ```

mod agent;
mod attachment;
mod config;
mod event;
mod recording;

pub use attachment::{
    ActionError, ActionReceipt, Adapter, Control, Driver, Error, Events, Running, attach,
};
pub use config::{Agent, Config, Recording, Target};
pub use event::{Event, Info, Problem, Rejection, SocketOperation, TransportFault};
