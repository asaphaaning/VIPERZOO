//! Detailed self character/equipment profile sent by the server.

use serde::Serialize;

use crate::{equipment, primitive::Opaque};

/// One legend/history row from the character sheet.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Legend {
    icon: u8,
    color: u8,
    text: String,
}

impl Legend {
    pub(crate) const fn new(icon: u8, color: u8, text: String) -> Self {
        Self { icon, color, text }
    }
}

/// Detailed self profile returned by clientbound opcode `0x39`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Character {
    armor_class: i8,
    damage: u8,
    hit: u8,
    clan: String,
    clan_title: String,
    title: String,
    spouse: String,
    group_enabled: bool,
    time_to_next_level: u32,
    class_name: String,
    equipment: Vec<equipment::Item>,
    exchange_enabled: bool,
    legend_header_reserved: u8,
    legends: Vec<Legend>,
    opaque_tail: Opaque,
}

impl Character {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        armor_class: i8,
        damage: u8,
        hit: u8,
        clan: String,
        clan_title: String,
        title: String,
        spouse: String,
        group_enabled: bool,
        time_to_next_level: u32,
        class_name: String,
        equipment: Vec<equipment::Item>,
        exchange_enabled: bool,
        legend_header_reserved: u8,
        legends: Vec<Legend>,
        tail: &[u8],
    ) -> Self {
        Self {
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
            opaque_tail: Opaque::from_slice(tail),
        }
    }

    /// Borrows all present equipment records in stable slot order.
    #[must_use]
    pub fn equipment(&self) -> &[equipment::Item] {
        &self.equipment
    }
}
