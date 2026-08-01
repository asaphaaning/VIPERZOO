//! Inventory-family structural decoders.

use crate::inventory;

use super::bytes::{Cursor, be_u16};

pub(super) fn item(body: &[u8]) -> Option<inventory::Item> {
    if body.len() < 22 || body[0] != 0x0f || body[1] == 0 {
        return None;
    }

    let mut cursor = Cursor::new(body.get(5..)?);
    let display_length = usize::from(cursor.take_u8()?);
    let display_name = text(cursor.take(display_length)?);
    let canonical_length = usize::from(cursor.take_u8()?);
    let canonical_name = text(cursor.take(canonical_length)?);
    let amount = cursor.take_u32()?;
    let stack_mode = cursor.take_u8()?;
    let durability = cursor.take_u32()?;
    let protected = cursor.take_u8()?;
    let owner_length = usize::from(cursor.take_u8()?);
    let owner = text(cursor.take(owner_length)?);

    Some(inventory::Item::new(
        body[1],
        be_u16(body, 2)?,
        body[4],
        display_name,
        canonical_name,
        amount,
        stack_mode,
        durability,
        protected,
        owner,
        cursor.remaining(),
    ))
}

pub(super) fn cleared(body: &[u8]) -> Option<inventory::Cleared> {
    if body.len() != 9 || body[0] != 0x10 || body[1] == 0 {
        return None;
    }

    Some(inventory::Cleared::new(body[1], body.get(2..)?))
}

fn text(bytes: &[u8]) -> String {
    encoding_rs::WINDOWS_1252
        .decode_without_bom_handling(bytes)
        .0
        .into()
}
