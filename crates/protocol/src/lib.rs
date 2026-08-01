//! Model already-decrypted `NexusTK` logical bodies.
//!
//! This crate begins only after a transport has delimited and decrypted a body.
//! It is deliberately not a socket codec: framing, encryption, sequencing,
//! retries, and recovery belong to a future wire boundary.
//!
//! [`decode`] has two deliberately different outcomes for unfamiliar input.
//! An unknown opcode remains a lossless [`packet::Unknown`], preserving evidence
//! for later research. A known opcode with an invalid layout is an [`Error`],
//! and therefore cannot mutate the world as a partially decoded packet.
//!
//! ```text
//! plaintext body
//! ├── known opcode + valid layout ──► typed client/server packet
//! ├── unknown opcode ───────────────► retained Unknown body
//! └── known opcode + invalid layout ► Error; no projected state
//! ```

mod decode;

pub mod action;
pub mod client;
pub mod combat;
pub mod direction;
pub mod entity;
pub mod equipment;
pub mod heartbeat;
pub mod inventory;
pub mod map;
pub mod message;
pub mod packet;
pub mod player;
pub mod primitive;
pub mod profile;
pub mod server;
pub mod spell;
pub mod travel;

pub use decode::{Error, decode};
