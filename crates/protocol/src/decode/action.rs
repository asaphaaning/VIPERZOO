//! Client action structural decoders.

use crate::{
    action,
    direction::Direction,
    primitive::{EntityId, MapId, Opaque, Position},
};

use super::bytes::{be_u16, be_u32};

pub(super) fn pickup(body: &[u8]) -> Option<action::Pickup> {
    (body.len() == 3 && body[0] == 0x07).then(|| action::Pickup::new(body[1], body[2]))
}

pub(super) fn disconnect(body: &[u8]) -> Option<action::Disconnect> {
    (body.len() == 2 && body[0] == 0x0b).then(|| action::Disconnect::new(body[1]))
}

pub(super) fn refresh(body: &[u8]) -> Option<action::Refresh> {
    (body.len() == 2 && body[0] == 0x38).then(|| action::Refresh::new(body[1]))
}

pub(super) fn use_inventory(body: &[u8]) -> Option<action::UseInventory> {
    if body.len() != 7 || body[0] != 0x1c || body[1] == 0 {
        return None;
    }

    Some(action::UseInventory::new(
        body[1],
        EntityId::new(be_u32(body, 2)?),
        body[6],
    ))
}

pub(super) fn movement(body: &[u8]) -> Option<action::Movement> {
    let direction = Direction::from_wire(*body.get(1)?)?;
    let last_walk = *body.get(2)?;
    let speed = *body.get(3)?;
    let origin = Position::new(be_u16(body, 4)?, be_u16(body, 6)?);

    match body.len() {
        17 => Some(action::Movement::WithMapRegion {
            direction,
            last_walk,
            speed,
            origin,
            region_origin: Position::new(be_u16(body, 8)?, be_u16(body, 10)?),
            region_width: *body.get(12)?,
            region_height: *body.get(13)?,
            checksum: be_u16(body, 14)?,
            opaque_tail: crate::primitive::Opaque::from_slice(body.get(16..17)?),
        }),
        10 => Some(action::Movement::Compact {
            direction,
            last_walk,
            speed,
            origin,
            opaque_tail: crate::primitive::Opaque::from_slice(body.get(8..10)?),
        }),
        _ => None,
    }
}

pub(super) fn obstruction(body: &[u8]) -> Option<action::Obstruction> {
    if body.len() != 7 {
        return None;
    }

    Some(action::Obstruction::new(
        Position::new(be_u16(body, 1)?, be_u16(body, 3)?),
        Direction::from_wire(*body.get(5)?)?,
        *body.get(6)?,
    ))
}

pub(super) fn facing(body: &[u8]) -> Option<action::Facing> {
    if body.len() != 3 {
        return None;
    }

    Some(action::Facing::new(
        Direction::from_wire(*body.get(1)?)?,
        *body.get(2)?,
    ))
}

pub(super) fn attack(body: &[u8]) -> Option<action::Attack> {
    if body.len() != 3 {
        return None;
    }

    Some(action::Attack::new(u16::from_le_bytes([
        *body.get(1)?,
        *body.get(2)?,
    ])))
}

pub(super) fn cast(body: &[u8]) -> Option<action::Cast> {
    if body.len() < 2 || *body.get(1)? == 0 {
        return None;
    }

    Some(action::Cast::new(
        *body.get(1)?,
        crate::primitive::Opaque::from_slice(body.get(2..)?),
    ))
}

pub(super) fn speech(body: &[u8]) -> Option<action::Speech> {
    let length = usize::from(*body.get(2)?);
    let text = body.get(3..)?;

    if body.first() != Some(&0x0e)
        || length.checked_add(1)? != text.len()
        || text.last() != Some(&0)
        || !text[..text.len().checked_sub(1)?].is_ascii()
    {
        return None;
    }

    let text = std::str::from_utf8(&text[..text.len() - 1]).ok()?.into();
    Some(action::Speech::new(body[1], text))
}

pub(super) fn interact(body: &[u8]) -> Option<action::Interact> {
    if body.len() != 7 || body[0] != 0x43 {
        return None;
    }

    Some(action::Interact::new(
        body[1],
        EntityId::new(be_u32(body, 2)?),
        body[6],
    ))
}

pub(super) fn dialog(body: &[u8]) -> Option<action::Dialog> {
    if body.len() < 9 || body[0] != 0x39 {
        return None;
    }

    let argument_length = usize::from(body[8]);
    let argument_end = 9_usize.checked_add(argument_length)?;
    let argument = body.get(9..argument_end)?;

    if !argument.is_ascii() {
        return None;
    }

    let argument = if argument.is_empty() {
        None
    } else {
        Some(std::str::from_utf8(argument).ok()?.into())
    };

    Some(action::Dialog::new(
        body[1],
        EntityId::new(be_u32(body, 2)?),
        body[7],
        argument,
        Opaque::from_slice(body.get(argument_end..)?),
    ))
}

pub(super) fn travel_selection(body: &[u8]) -> Option<action::TravelSelection> {
    if body.len() != 8 || body[0] != 0x3f {
        return None;
    }

    Some(action::TravelSelection::new(
        MapId::new(be_u16(body, 1)?),
        Position::new(be_u16(body, 3)?, be_u16(body, 5)?),
        body[7],
    ))
}
