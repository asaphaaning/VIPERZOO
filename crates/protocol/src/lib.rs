//! Model and decode already-decrypted `NexusTK` logical bodies.
//!
//! This crate begins only after the client has delimited and decrypted a body.
//! [`codec::Codec`] consumes the direction and length metadata supplied by that
//! plaintext boundary. Encryption, socket framing, sequencing, retries, and
//! recovery remain below this layer.
//!
//! Decoding has two deliberately different outcomes for unfamiliar input. An
//! unknown opcode remains a lossless [`packet::Unknown`], preserving evidence
//! for later research. A known opcode with an invalid layout is a
//! [`codec::Error`], and therefore cannot mutate the world as a partially
//! decoded packet.
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
pub mod codec;
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
