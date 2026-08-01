//! Heartbeat-family structural decoders.

use crate::heartbeat as protocol;
use crate::primitive::Opaque;

pub(super) fn ping(body: &[u8]) -> Option<protocol::Ping> {
    if body.len() != 13 {
        return None;
    }

    Some(protocol::Ping::new(
        protocol::Challenge::new(body.get(1..9)?.try_into().ok()?),
        Opaque::from_slice(body.get(9..13)?),
    ))
}

pub(super) fn pong(body: &[u8]) -> Option<protocol::Pong> {
    if body.len() != 18 {
        return None;
    }

    Some(protocol::Pong::new(
        protocol::Challenge::new(body.get(1..9)?.try_into().ok()?),
        body.get(9..14)?.try_into().ok()?,
        (u32::from(*body.get(14)?) << 16)
            | (u32::from(*body.get(15)?) << 8)
            | u32::from(*body.get(16)?),
        *body.get(17)?,
    ))
}
