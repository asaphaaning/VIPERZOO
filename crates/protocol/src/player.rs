//! Player position and flag-controlled status observations.

use serde::Serialize;

use crate::primitive::{Opaque, Position};

/// A protocol-native player localization observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "evidence", rename_all = "snake_case")]
pub enum Location {
    /// A login, map-entry, or refresh seed with no movement counter.
    Seed {
        /// The player's current tile.
        position: Position,
        /// The viewport anchor.
        viewport: Position,
        /// The protocol reserved value.
        reserved: u8,
        /// Retained workspace bytes.
        opaque_tail: Opaque,
    },
    /// A movement result correlated with a client `LastWalk` value.
    Authoritative {
        /// The server-authoritative player tile.
        position: Position,
        /// The viewport anchor.
        viewport: Position,
        /// The observed movement result status.
        status: u8,
        /// The correlated wrapping client movement counter.
        last_walk: u8,
        /// Retained workspace bytes.
        opaque_tail: Opaque,
    },
}

impl Location {
    /// Returns the observed player tile.
    #[must_use]
    pub const fn position(&self) -> Position {
        match self {
            Self::Seed { position, .. } | Self::Authoritative { position, .. } => *position,
        }
    }

    /// Returns the observed viewport anchor.
    #[must_use]
    pub const fn viewport(&self) -> Position {
        match self {
            Self::Seed { viewport, .. } | Self::Authoritative { viewport, .. } => *viewport,
        }
    }
}

/// The flag byte controlling the blocks present in a [`Status`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct StatusFlags(u8);

impl StatusFlags {
    pub(crate) const FULL: u8 = 0x40;
    pub(crate) const RESOURCES: u8 = 0x20;
    pub(crate) const ECONOMY: u8 = 0x10;
    pub(crate) const ALWAYS: u8 = 0x08;

    pub(crate) const fn new(value: u8) -> Option<Self> {
        if value & Self::ALWAYS != 0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub(crate) const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    /// Returns the original wire flags.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

/// One composable player status packet.
///
/// Private fields guarantee that each optional block agrees with [`StatusFlags`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Status {
    flags: StatusFlags,
    full: Option<FullStatus>,
    resources: Option<Resources>,
    economy: Option<Economy>,
    condition: Condition,
    opaque_tail: Opaque,
}

impl Status {
    pub(crate) fn new(
        flags: StatusFlags,
        full: Option<FullStatus>,
        resources: Option<Resources>,
        economy: Option<Economy>,
        condition: Condition,
        opaque_tail: Opaque,
    ) -> Option<Self> {
        let coherent = flags.contains(StatusFlags::FULL) == full.is_some()
            && flags.contains(StatusFlags::RESOURCES) == resources.is_some()
            && flags.contains(StatusFlags::ECONOMY) == economy.is_some();

        coherent.then_some(Self {
            flags,
            full,
            resources,
            economy,
            condition,
            opaque_tail,
        })
    }

    /// Returns the controlling wire flags.
    #[must_use]
    pub const fn flags(&self) -> StatusFlags {
        self.flags
    }

    /// Returns the full-stat block when transmitted.
    #[must_use]
    pub const fn full(&self) -> Option<&FullStatus> {
        self.full.as_ref()
    }

    /// Returns the absolute VITA/mana block when transmitted.
    #[must_use]
    pub const fn resources(&self) -> Option<Resources> {
        self.resources
    }

    /// Returns the experience/money block when transmitted.
    #[must_use]
    pub const fn economy(&self) -> Option<Economy> {
        self.economy
    }

    /// Returns the always-present condition block.
    #[must_use]
    pub const fn condition(&self) -> Condition {
        self.condition
    }

    /// Returns retained workspace bytes.
    #[must_use]
    pub const fn opaque_tail(&self) -> &Opaque {
        &self.opaque_tail
    }
}

/// Character properties present in a full status update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FullStatus {
    nation: u8,
    totem: u8,
    level: u8,
    max_vita: u32,
    max_mana: u32,
    might: u8,
    will: u8,
    grace: u8,
    max_inventory: u8,
}

impl FullStatus {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        nation: u8,
        totem: u8,
        level: u8,
        max_vita: u32,
        max_mana: u32,
        might: u8,
        will: u8,
        grace: u8,
        max_inventory: u8,
    ) -> Self {
        Self {
            nation,
            totem,
            level,
            max_vita,
            max_mana,
            might,
            will,
            grace,
            max_inventory,
        }
    }

    /// Returns the character level.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// Returns maximum VITA.
    #[must_use]
    pub const fn max_vita(&self) -> u32 {
        self.max_vita
    }

    /// Returns maximum mana.
    #[must_use]
    pub const fn max_mana(&self) -> u32 {
        self.max_mana
    }

    /// Returns the nation value.
    #[must_use]
    pub const fn nation(&self) -> u8 {
        self.nation
    }

    /// Returns the totem value.
    #[must_use]
    pub const fn totem(&self) -> u8 {
        self.totem
    }

    /// Returns might.
    #[must_use]
    pub const fn might(&self) -> u8 {
        self.might
    }

    /// Returns will.
    #[must_use]
    pub const fn will(&self) -> u8 {
        self.will
    }

    /// Returns grace.
    #[must_use]
    pub const fn grace(&self) -> u8 {
        self.grace
    }

    /// Returns the maximum inventory slot count.
    #[must_use]
    pub const fn max_inventory(&self) -> u8 {
        self.max_inventory
    }
}

/// Absolute player resource values carried together.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Resources {
    vita: u32,
    mana: u32,
}

impl Resources {
    pub(crate) const fn new(vita: u32, mana: u32) -> Self {
        Self { vita, mana }
    }

    /// Returns current VITA.
    #[must_use]
    pub const fn vita(self) -> u32 {
        self.vita
    }

    /// Returns current mana.
    #[must_use]
    pub const fn mana(self) -> u32 {
        self.mana
    }
}

/// Experience and money values carried together.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Economy {
    experience: u32,
    money: u32,
    experience_percent: u8,
}

impl Economy {
    pub(crate) const fn new(experience: u32, money: u32, experience_percent: u8) -> Self {
        Self {
            experience,
            money,
            experience_percent,
        }
    }

    /// Returns total experience.
    #[must_use]
    pub const fn experience(self) -> u32 {
        self.experience
    }

    /// Returns carried money.
    #[must_use]
    pub const fn money(self) -> u32 {
        self.money
    }

    /// Returns the displayed experience percentage.
    #[must_use]
    pub const fn experience_percent(self) -> u8 {
        self.experience_percent
    }
}

/// Always-present player condition/settings values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Condition {
    drunk: u8,
    blind: u8,
    notification_flags: u8,
    setting_flags: u32,
}

impl Condition {
    pub(crate) const fn new(
        drunk: u8,
        blind: u8,
        notification_flags: u8,
        setting_flags: u32,
    ) -> Self {
        Self {
            drunk,
            blind,
            notification_flags,
            setting_flags,
        }
    }

    /// Returns the drunk status byte.
    #[must_use]
    pub const fn drunk(self) -> u8 {
        self.drunk
    }

    /// Returns the blind status byte.
    #[must_use]
    pub const fn blind(self) -> u8 {
        self.blind
    }

    /// Returns client notification flags.
    #[must_use]
    pub const fn notification_flags(self) -> u8 {
        self.notification_flags
    }

    /// Returns client setting flags.
    #[must_use]
    pub const fn setting_flags(self) -> u32 {
        self.setting_flags
    }
}
