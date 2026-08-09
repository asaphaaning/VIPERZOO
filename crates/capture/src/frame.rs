//! Typed evidence values written by the capture stream codec.

use viperzoo_protocol::direction::Flow;

/// One timestamped evidence frame at the plaintext acquisition boundary.
///
/// Packet frames retain raw bytes because recording precedes structural packet
/// promotion. This lets malformed and unknown bodies remain available for
/// later protocol research.
#[derive(Clone, Copy, Debug)]
pub enum Frame<'a> {
    /// A new client attachment began producing evidence.
    SessionStarted {
        /// Operating-system process identifier of the attached client.
        target_pid: u32,
        /// Milliseconds since the Unix epoch at acquisition time.
        captured_unix_ms: u128,
    },
    /// One bounded plaintext protocol body.
    Packet {
        /// Direction in which the client observed the body.
        flow: Flow,
        /// Complete unencrypted logical body.
        body: &'a [u8],
        /// Operating-system thread that crossed the plaintext hook.
        thread_id: Option<u32>,
        /// Milliseconds since the Unix epoch at acquisition time.
        captured_unix_ms: u128,
    },
    /// The active socket closed below the plaintext hooks.
    TransportClosed {
        /// Adapter-specific evidence describing the closure.
        source: &'a str,
        /// Milliseconds since the Unix epoch at acquisition time.
        captured_unix_ms: u128,
    },
    /// A socket operation failed below the plaintext hooks.
    TransportFault {
        /// Closed vocabulary of socket operations observed by the tap.
        operation: Operation,
        /// Platform socket error code.
        code: i32,
        /// Milliseconds since the Unix epoch at acquisition time.
        captured_unix_ms: u128,
    },
}

/// A socket operation represented by a capture fault frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Receive bytes from the remote peer.
    Receive,
    /// Send bytes to the remote peer.
    Send,
}

impl Operation {
    /// Canonical operation vocabulary in declaration order.
    pub const VARIANTS: [Self; 2] = [Self::Receive, Self::Send];
}
