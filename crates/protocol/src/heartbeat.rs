//! Client/server heartbeat observations.

use serde::Serialize;

use crate::primitive::Opaque;

/// The eight-byte server challenge echoed by the client.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Challenge([u8; 8]);

impl Challenge {
    pub(crate) const fn new(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Returns the challenge bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 8] {
        self.0
    }
}

/// One server heartbeat challenge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Ping {
    challenge: Challenge,
    opaque_tail: Opaque,
}

impl Ping {
    pub(crate) fn new(challenge: Challenge, opaque_tail: Opaque) -> Self {
        Self {
            challenge,
            opaque_tail,
        }
    }

    /// Returns the challenge that must be echoed.
    #[must_use]
    pub const fn challenge(&self) -> Challenge {
        self.challenge
    }
}

/// One client heartbeat response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Pong {
    challenge: Challenge,
    reserved: [u8; 5],
    client_tick_low24: u32,
    terminator: u8,
}

impl Pong {
    pub(crate) const fn new(
        challenge: Challenge,
        reserved: [u8; 5],
        client_tick_low24: u32,
        terminator: u8,
    ) -> Self {
        Self {
            challenge,
            reserved,
            client_tick_low24,
            terminator,
        }
    }

    /// Returns the echoed challenge.
    #[must_use]
    pub const fn challenge(&self) -> Challenge {
        self.challenge
    }

    /// Returns the client tick encoded in the low 24 bits.
    #[must_use]
    pub const fn client_tick_low24(&self) -> u32 {
        self.client_tick_low24
    }

    /// Returns the reserved response bytes.
    #[must_use]
    pub const fn reserved(&self) -> [u8; 5] {
        self.reserved
    }

    /// Returns the trailing terminator.
    #[must_use]
    pub const fn terminator(&self) -> u8 {
        self.terminator
    }
}
