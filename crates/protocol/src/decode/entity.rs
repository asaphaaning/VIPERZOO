//! Entity-family structural decoders.

use crate::direction::Direction;
use crate::entity as protocol;
use crate::primitive::{EntityId, Opaque, Position};

use super::bytes::{Cursor, be_u16, be_u32};

pub(super) fn appearances(body: &[u8]) -> Option<protocol::Appearances> {
    let count = usize::from(be_u16(body, 1)?);

    if count == 0 || count > 512 {
        return None;
    }

    let mut cursor = Cursor::new(body.get(3..)?);
    let mut appearances = Vec::with_capacity(count);

    for _ in 0..count {
        let prefix = cursor.peek(15)?;
        let animation_count = usize::from(*prefix.get(14)?);
        let record_length = 15usize.checked_add(animation_count.checked_mul(4)?)?;
        let record = cursor.take(record_length)?;
        let direction = Direction::from_wire(*record.get(12)?)?;
        let mut animations = Vec::with_capacity(animation_count);

        for index in 0..animation_count {
            let offset = 15 + index * 4;

            animations.push(protocol::Animation::new(
                be_u16(record, offset)?,
                be_u16(record, offset + 2)?,
            ));
        }

        appearances.push(protocol::Appearance::new(
            EntityId::new(be_u32(record, 5)?),
            Position::new(be_u16(record, 0)?, be_u16(record, 2)?),
            protocol::Kind::from_wire(*record.get(4)?),
            be_u16(record, 9)?,
            *record.get(11)?,
            direction,
            *record.get(13)?,
            animations,
        ));
    }

    let pass_flag = cursor.take_u8()?;

    if cursor.remaining().len() > 8 {
        return None;
    }

    Some(protocol::Appearances::new(
        appearances,
        pass_flag,
        Opaque::from_slice(cursor.remaining()),
    ))
}

pub(super) fn movement(body: &[u8]) -> Option<protocol::Movement> {
    if body.len() != 15 {
        return None;
    }

    Some(protocol::Movement::new(
        EntityId::new(be_u32(body, 1)?),
        Position::new(be_u16(body, 5)?, be_u16(body, 7)?),
        Direction::from_wire(*body.get(9)?)?,
        Opaque::from_slice(body.get(10..15)?),
    ))
}

pub(super) fn removal(body: &[u8]) -> Option<protocol::Removal> {
    if body.len() != 9 {
        return None;
    }

    Some(protocol::Removal::new(
        EntityId::new(be_u32(body, 1)?),
        Opaque::from_slice(body.get(5..9)?),
    ))
}

pub(super) fn control(body: &[u8]) -> Option<protocol::Control> {
    if body.len() != 6 {
        return None;
    }

    Some(protocol::Control::new(
        *body.get(1)?,
        Opaque::from_slice(body.get(2..6)?),
    ))
}
