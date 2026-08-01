//! Project connection evidence without inventing a liveness policy.
//!
//! The world sees two distinct kinds of evidence: plaintext heartbeat bodies
//! and a socket close observed below the packet boundary. They must remain
//! distinct because the client normally answers heartbeats itself, while an
//! orderly remote close has no plaintext body at all.
//!
//! [`Heartbeat`] correlates observed challenges and responses; it does not
//! generate a keepalive or claim that a matching response guarantees a healthy
//! session. [`Connection`] records a client close body separately from a
//! transport close so callers can reason about what was actually observed.
//!
//! ```text
//! server challenge ──► remember challenge ──► later response may match
//! client close body ──► Connection::CloseObserved
//! recv == 0          ──► Connection::TransportClosed
//! ```

use serde::Serialize;
use viperzoo_protocol::heartbeat;

/// Client session lifecycle projected from the plaintext protocol boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    /// The current attachment epoch has not requested termination.
    #[default]
    Active,
    /// The client emitted canonical close body `0x0B`.
    ///
    /// This records the wire fact without attributing intent: the client can
    /// emit it after an operator exit, a local idle watchdog, or a server boot.
    CloseObserved,
    /// The client's active game socket was closed without a plaintext `0x0B`.
    ///
    /// In particular, an orderly remote shutdown makes `recv` return zero and
    /// closes the socket below the plaintext packet boundary.
    TransportClosed,
}

/// Correlated heartbeat state.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Heartbeat {
    challenges_received: u64,
    pongs_observed: u64,
    matched_pongs: u64,
    last_challenge: Option<heartbeat::Challenge>,
    last_client_tick_low24: Option<u32>,
}

impl Heartbeat {
    /// Returns the number of server challenges observed.
    #[must_use]
    pub const fn challenges_received(&self) -> u64 {
        self.challenges_received
    }

    /// Returns the number of client responses observed.
    #[must_use]
    pub const fn pongs_observed(&self) -> u64 {
        self.pongs_observed
    }

    /// Returns the number of responses matching the latest challenge.
    #[must_use]
    pub const fn matched_pongs(&self) -> u64 {
        self.matched_pongs
    }

    pub(crate) fn observe_ping(&mut self, ping: &heartbeat::Ping) {
        self.challenges_received += 1;
        self.last_challenge = Some(ping.challenge());
    }

    pub(crate) fn observe_pong(&mut self, pong: &heartbeat::Pong) {
        self.pongs_observed += 1;
        self.last_client_tick_low24 = Some(pong.client_tick_low24());

        if self.last_challenge == Some(pong.challenge()) {
            self.matched_pongs += 1;
        }
    }
}
