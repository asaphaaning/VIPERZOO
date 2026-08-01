//! Quarantine untrusted capture rows before they reach the world.
//!
//! The JSONL vocabulary, hexadecimal text, declared lengths, and diagnostic
//! wording stop here. [`record::decode`] classifies every line as a typed
//! packet, a meaningful non-packet record, an intentional skip, or a
//! [`record::Diagnostic`].
//!
//! This is stricter than a generic parser: malformed representations of known
//! packets are retained as evidence but never become an engine observation.
//! Successful packet records use the same [`viperzoo_protocol::packet::Packet`]
//! type as live acquisition.

pub mod line;
pub mod record;
