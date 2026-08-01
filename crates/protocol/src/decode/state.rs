//! Spellbook, combat, and server-message families.

use crate::{combat, message, primitive::EntityId, spell};

use super::bytes::{be_u16, be_u32};

pub(super) fn spellbook(body: &[u8]) -> Option<spell::Entry> {
    if body.len() < 5 || body[0] != 0x17 || body[1] == 0 {
        return None;
    }

    let name_length = usize::from(body[3]);
    let name_start = 4;
    let question_length_at = name_start + name_length;
    let question_length = usize::from(*body.get(question_length_at)?);
    let question_start = question_length_at + 1;
    let tail_start = question_start.checked_add(question_length)?;

    Some(spell::Entry::new(
        body[1],
        body[2],
        text(body.get(name_start..question_length_at)?),
        text(body.get(question_start..tail_start)?),
        body.get(tail_start..)?,
    ))
}

pub(super) fn actor_action(body: &[u8]) -> Option<combat::Action> {
    if !(9..=17).contains(&body.len()) || body[0] != 0x1a {
        return None;
    }

    Some(combat::Action::new(
        EntityId::new(be_u32(body, 1)?),
        body[5],
        body[6],
        body[7],
        body[8],
        &body[9..],
    ))
}

pub(super) fn actor_vitality(body: &[u8]) -> Option<combat::Vitality> {
    if body.len() != 15 || body[0] != 0x13 {
        return None;
    }

    Some(combat::Vitality::new(
        EntityId::new(be_u32(body, 1)?),
        body[5],
        body[6],
        i32::from_be_bytes(body.get(7..11)?.try_into().ok()?),
        &body[11..],
    ))
}

pub(super) fn message(body: &[u8]) -> Option<message::Message> {
    if body.len() < 4 || body[0] != 0x0a {
        return None;
    }

    let length = usize::from(be_u16(body, 2)?);
    let end = 4_usize.checked_add(length)?;

    if end > body.len() || body.len() - end > 8 {
        return None;
    }

    Some(message::Message::new(
        body[1],
        text(&body[4..end]),
        &body[end..],
    ))
}

fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| cp1252(*byte)).collect()
}

fn cp1252(byte: u8) -> char {
    const EXTENDED: [char; 32] = [
        '€', '\u{0081}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{008d}', 'Ž',
        '\u{008f}', '\u{0090}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{009d}', 'ž', 'Ÿ',
    ];

    match byte {
        0x80..=0x9f => EXTENDED[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}
