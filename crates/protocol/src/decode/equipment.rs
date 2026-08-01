//! Equipment-family structural decoders.

use crate::equipment;

use super::bytes::{Cursor, be_u16};

pub(super) fn item(body: &[u8]) -> Option<equipment::Item> {
    if body.len() < 15 || body[0] != 0x37 || body[1] == 0 {
        return None;
    }

    let mut cursor = Cursor::new(body.get(5..)?);
    let display_length = usize::from(cursor.take_u8()?);
    let display_name = text(cursor.take(display_length)?);
    let canonical_length = usize::from(cursor.take_u8()?);
    let canonical_name = text(cursor.take(canonical_length)?);
    let durability = cursor.take_u32()?;

    Some(equipment::Item::new(
        body[1],
        be_u16(body, 2)?,
        body[4],
        display_name,
        canonical_name,
        durability,
        cursor.remaining(),
    ))
}

pub(super) fn cleared(body: &[u8]) -> Option<equipment::Cleared> {
    if body.len() != 6 || body[0] != 0x38 || body[1] == 0 {
        return None;
    }

    Some(equipment::Cleared::new(body[1], body.get(2..)?))
}

fn text(bytes: &[u8]) -> String {
    encoding_rs::WINDOWS_1252
        .decode_without_bom_handling(bytes)
        .0
        .into()
}
