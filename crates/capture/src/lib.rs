//! Quarantine untrusted capture rows before they reach the world.
//!
//! The JSONL vocabulary, hexadecimal text, declared lengths, and diagnostic
//! wording stop here. [`codec::Codec`] frames and classifies the byte stream as
//! typed packets, meaningful non-packet records, intentional skips, or
//! [`record::Diagnostic`] values. The same codec writes typed [`frame::Frame`]
//! evidence, keeping recording and replay on one vocabulary.
//!
//! This is stricter than a generic parser: malformed representations of known
//! packets are retained as evidence but never become an engine observation.
//! Successful packet records use the same [`viperzoo_protocol::packet::Packet`]
//! type as live acquisition.

pub mod codec;
pub mod frame;
pub mod line;
pub mod record;
