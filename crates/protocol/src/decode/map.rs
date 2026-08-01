//! Map-family structural decoders.

use encoding_rs::WINDOWS_1252;

use crate::map as protocol;
use crate::primitive::{MapId, Opaque, Position};

use super::bytes::be_u16;

pub(super) fn context(body: &[u8]) -> Option<protocol::Context> {
    let title_length = usize::from(*body.get(9)?);
    let title_end = 10usize.checked_add(title_length)?;
    let core_length = title_end.checked_add(2)?;

    if body.len() < core_length || body.len() - core_length > 8 {
        return None;
    }

    let dimensions = protocol::Dimensions::new(be_u16(body, 3)?, be_u16(body, 5)?)?;
    let (title, _, _) = WINDOWS_1252.decode(body.get(10..title_end)?);

    Some(protocol::Context::new(
        MapId::new(be_u16(body, 1)?),
        dimensions,
        *body.get(7)?,
        *body.get(8)?,
        title.into_owned(),
        be_u16(body, title_end)?,
        Opaque::from_slice(body.get(core_length..)?),
    ))
}

pub(super) fn region(body: &[u8]) -> Option<protocol::Region> {
    let size = protocol::RegionSize::new(*body.get(6)?, *body.get(7)?)?;
    let core_length = 8usize.checked_add(size.tile_count().checked_mul(6)?)?;

    if body.len() < core_length || body.len() - core_length > 8 {
        return None;
    }

    let mut tiles = Vec::with_capacity(size.tile_count());

    for offset in (8..core_length).step_by(6) {
        tiles.push(protocol::Tile::new(
            be_u16(body, offset)?,
            be_u16(body, offset + 2)?,
            be_u16(body, offset + 4)?,
        ));
    }

    protocol::Region::new(
        Position::new(be_u16(body, 2)?, be_u16(body, 4)?),
        size,
        tiles,
        Opaque::from_slice(body.get(core_length..)?),
    )
}
