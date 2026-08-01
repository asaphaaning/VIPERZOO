//! Player-family structural decoders.

use crate::player as protocol;
use crate::primitive::{Opaque, Position};

use super::bytes::{Cursor, be_u16, be_u32};

pub(super) fn location(body: &[u8]) -> Option<protocol::Location> {
    match (body.first(), body.len()) {
        (Some(0x04), 13) => Some(protocol::Location::Seed {
            position: Position::new(be_u16(body, 1)?, be_u16(body, 3)?),
            viewport: Position::new(be_u16(body, 5)?, be_u16(body, 7)?),
            reserved: *body.get(9)?,
            opaque_tail: Opaque::from_slice(body.get(10..13)?),
        }),
        (Some(0x26), 15) => Some(protocol::Location::Authoritative {
            status: *body.get(1)?,
            position: Position::new(be_u16(body, 2)?, be_u16(body, 4)?),
            viewport: Position::new(be_u16(body, 6)?, be_u16(body, 8)?),
            last_walk: *body.get(10)?,
            opaque_tail: Opaque::from_slice(body.get(11..15)?),
        }),
        _ => None,
    }
}

pub(super) fn status(body: &[u8]) -> Option<protocol::Status> {
    let flags = protocol::StatusFlags::new(*body.get(1)?)?;
    let mut cursor = Cursor::new(body.get(2..)?);

    let full = if flags.contains(protocol::StatusFlags::FULL) {
        let block = cursor.take(29)?;

        Some(protocol::FullStatus::new(
            *block.get(1)?,
            *block.get(2)?,
            *block.get(4)?,
            be_u32(block, 5)?,
            be_u32(block, 9)?,
            *block.get(13)?,
            *block.get(14)?,
            *block.get(17)?,
            *block.get(28)?,
        ))
    } else {
        None
    };

    let resources = if flags.contains(protocol::StatusFlags::RESOURCES) {
        Some(protocol::Resources::new(
            cursor.take_u32()?,
            cursor.take_u32()?,
        ))
    } else {
        None
    };

    let economy = if flags.contains(protocol::StatusFlags::ECONOMY) {
        Some(protocol::Economy::new(
            cursor.take_u32()?,
            cursor.take_u32()?,
            cursor.take_u8()?,
        ))
    } else {
        None
    };

    let condition = cursor.take(11)?;
    let condition = protocol::Condition::new(
        *condition.first()?,
        *condition.get(1)?,
        *condition.get(5)?,
        be_u32(condition, 7)?,
    );

    protocol::Status::new(
        flags,
        full,
        resources,
        economy,
        condition,
        Opaque::from_slice(cursor.remaining()),
    )
}
