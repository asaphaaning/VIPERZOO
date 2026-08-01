//! Character-profile structural decoder.

use crate::{equipment, profile};

use super::bytes::Cursor;

const EQUIPMENT_SLOTS: u8 = 14;

pub(super) fn character(body: &[u8]) -> Option<profile::Character> {
    if body.len() < 158 || body[0] != 0x39 {
        return None;
    }

    let armor_class = i8::from_be_bytes([body[1]]);
    let damage = body[2];
    let hit = body[3];
    let mut cursor = Cursor::new(body.get(4..)?);
    let clan = text_field(&mut cursor)?;
    let clan_title = text_field(&mut cursor)?;
    let title = text_field(&mut cursor)?;
    let spouse = text_field(&mut cursor)?;
    let group_enabled = cursor.take_u8()? != 0;
    let time_to_next_level = cursor.take_u32()?;
    let class_name = text_field(&mut cursor)?;
    let mut equipment = Vec::new();

    for slot in 1..=EQUIPMENT_SLOTS {
        let icon = cursor.take(2)?;
        let icon_id = u16::from_be_bytes(icon.try_into().ok()?);
        let icon_color = cursor.take_u8()?;
        let display_name = text_field(&mut cursor)?;
        let canonical_name = text_field(&mut cursor)?;
        let durability = cursor.take_u32()?;
        let record_flag = cursor.take_u8()?;

        if icon_id != 0 {
            equipment.push(equipment::Item::new(
                slot,
                icon_id,
                icon_color,
                display_name,
                canonical_name,
                durability,
                &[record_flag],
            ));
        }
    }

    let exchange_enabled = cursor.take_u8()? != 0;
    let legend_header_reserved = cursor.take_u8()?;
    let legend_count = cursor.take(2)?;
    let legend_count = u16::from_be_bytes(legend_count.try_into().ok()?);
    let mut legends = Vec::with_capacity(usize::from(legend_count));

    for _ in 0..legend_count {
        legends.push(profile::Legend::new(
            cursor.take_u8()?,
            cursor.take_u8()?,
            text_field(&mut cursor)?,
        ));
    }

    if cursor.remaining().len() > 8 {
        return None;
    }

    Some(profile::Character::new(
        armor_class,
        damage,
        hit,
        clan,
        clan_title,
        title,
        spouse,
        group_enabled,
        time_to_next_level,
        class_name,
        equipment,
        exchange_enabled,
        legend_header_reserved,
        legends,
        cursor.remaining(),
    ))
}

fn text_field(cursor: &mut Cursor<'_>) -> Option<String> {
    let length = usize::from(cursor.take_u8()?);
    Some(
        encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(cursor.take(length)?)
            .0
            .into(),
    )
}
