//! Projected player location and resources.

use serde::Serialize;
use viperzoo_adapter_api::resource;
use viperzoo_protocol::{action, direction::Direction, player as protocol, primitive::Position};

use crate::knowledge::{Knowledge, Source};
use crate::revision::Revision;

/// Player localization with the evidence shape that established it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Location {
    /// No localization packet has been observed in this attachment epoch.
    #[default]
    Unknown,
    /// A login, map-entry, or refresh packet seeded the location.
    Seeded {
        /// Player tile.
        position: Position,
        /// Viewport anchor.
        viewport: Position,
        /// World revision of the seed.
        revision: Revision,
    },
    /// The native client reported its current coordinate through movement state.
    ClientReported {
        /// Client-local player tile after applying the latest evidence.
        position: Position,
        /// Viewport anchor retained from the latest server location, if known.
        viewport: Option<Position>,
        /// Client evidence that established the coordinate.
        evidence: ClientEvidence,
        /// World revision of the client report.
        revision: Revision,
    },
    /// A movement result supplied authoritative coordinates and correlation.
    Authoritative {
        /// Player tile.
        position: Position,
        /// Viewport anchor.
        viewport: Position,
        /// Movement result status.
        status: u8,
        /// Correlated wrapping client movement counter.
        last_walk: u8,
        /// World revision of the result.
        revision: Revision,
    },
}

/// Client-local evidence used between authoritative server position packets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientEvidence {
    /// A movement request predicts its one-tile destination.
    Movement {
        /// Position reported before applying the step.
        origin: Position,
        /// Wrapping movement counter.
        last_walk: u8,
    },
    /// An obstruction report restores the unchanged origin.
    Obstruction {
        /// Rejected direction from the unchanged tile.
        direction: Direction,
    },
}

impl Location {
    pub(crate) const fn from_protocol(location: &protocol::Location, revision: Revision) -> Self {
        match location {
            protocol::Location::Seed {
                position, viewport, ..
            } => Self::Seeded {
                position: *position,
                viewport: *viewport,
                revision,
            },
            protocol::Location::Authoritative {
                position,
                viewport,
                status,
                last_walk,
                ..
            } => Self::Authoritative {
                position: *position,
                viewport: *viewport,
                status: *status,
                last_walk: *last_walk,
                revision,
            },
        }
    }

    /// Returns the player tile if known.
    #[must_use]
    pub const fn position(&self) -> Option<Position> {
        match self {
            Self::Unknown => None,
            Self::Seeded { position, .. }
            | Self::ClientReported { position, .. }
            | Self::Authoritative { position, .. } => Some(*position),
        }
    }

    const fn viewport(&self) -> Option<Position> {
        match self {
            Self::Unknown => None,
            Self::Seeded { viewport, .. } | Self::Authoritative { viewport, .. } => Some(*viewport),
            Self::ClientReported { viewport, .. } => *viewport,
        }
    }

    fn same_value(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unknown, Self::Unknown) => true,
            (
                Self::Seeded {
                    position, viewport, ..
                },
                Self::Seeded {
                    position: other_position,
                    viewport: other_viewport,
                    ..
                },
            ) => position == other_position && viewport == other_viewport,
            (
                Self::ClientReported {
                    position,
                    viewport,
                    evidence,
                    ..
                },
                Self::ClientReported {
                    position: other_position,
                    viewport: other_viewport,
                    evidence: other_evidence,
                    ..
                },
            ) => {
                position == other_position
                    && viewport == other_viewport
                    && evidence == other_evidence
            }
            (
                Self::Authoritative {
                    position,
                    viewport,
                    status,
                    last_walk,
                    ..
                },
                Self::Authoritative {
                    position: other_position,
                    viewport: other_viewport,
                    status: other_status,
                    last_walk: other_last_walk,
                    ..
                },
            ) => {
                position == other_position
                    && viewport == other_viewport
                    && status == other_status
                    && last_walk == other_last_walk
            }
            _ => false,
        }
    }
}

/// Current and maximum values for one player resource.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Pool {
    current: Knowledge<u32>,
    maximum: Knowledge<u32>,
}

impl Pool {
    /// Returns current resource knowledge.
    #[must_use]
    pub const fn current(&self) -> &Knowledge<u32> {
        &self.current
    }

    /// Returns maximum resource knowledge.
    #[must_use]
    pub const fn maximum(&self) -> &Knowledge<u32> {
        &self.maximum
    }

    fn observe_current(&mut self, value: u32, revision: Revision, source: Source) -> bool {
        if self.current.value() == Some(&value) && self.current.source() == Some(source) {
            return false;
        }

        self.current = Knowledge::observed(value, revision, source);
        true
    }

    fn observe_maximum(&mut self, value: u32, revision: Revision, source: Source) -> bool {
        if self.maximum.value() == Some(&value) && self.maximum.source() == Some(source) {
            return false;
        }

        self.maximum = Knowledge::observed(value, revision, source);
        true
    }

    fn seed(&mut self, pool: resource::Pool, revision: Revision) -> bool {
        let mut changed = false;

        if self.current.source() != Some(Source::PlayerStatus) {
            changed |= self.observe_current(pool.current(), revision, Source::ClientMemoryBuild752);
        }

        if self.maximum.source() != Some(Source::PlayerStatus) {
            changed |= self.observe_maximum(pool.maximum(), revision, Source::ClientMemoryBuild752);
        }

        changed
    }
}

/// Player VITA and mana projection.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Resources {
    vita: Pool,
    mana: Pool,
}

/// Player economy values from the latest server status update.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Economy {
    experience: Knowledge<u32>,
    money: Knowledge<u32>,
    experience_percent: Knowledge<u8>,
}

impl Economy {
    /// Returns total-experience knowledge.
    #[must_use]
    pub const fn experience(&self) -> &Knowledge<u32> {
        &self.experience
    }

    /// Returns carried-money knowledge.
    #[must_use]
    pub const fn money(&self) -> &Knowledge<u32> {
        &self.money
    }

    /// Returns displayed experience-percentage knowledge.
    #[must_use]
    pub const fn experience_percent(&self) -> &Knowledge<u8> {
        &self.experience_percent
    }
}

impl Resources {
    /// Returns VITA knowledge.
    #[must_use]
    pub const fn vita(&self) -> &Pool {
        &self.vita
    }

    /// Returns mana knowledge.
    #[must_use]
    pub const fn mana(&self) -> &Pool {
        &self.mana
    }
}

/// Immutable projected player state.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct State {
    location: Location,
    facing: Knowledge<Direction>,
    resources: Resources,
    level: Knowledge<u8>,
    inventory_capacity: Knowledge<u8>,
    economy: Economy,
}

impl State {
    /// Returns localization knowledge.
    #[must_use]
    pub const fn location(&self) -> &Location {
        &self.location
    }

    /// Returns the latest client-reported facing direction.
    #[must_use]
    pub const fn facing(&self) -> &Knowledge<Direction> {
        &self.facing
    }

    /// Returns player resource knowledge.
    #[must_use]
    pub const fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Returns character-level knowledge.
    #[must_use]
    pub const fn level(&self) -> &Knowledge<u8> {
        &self.level
    }

    /// Returns maximum-inventory-slot knowledge.
    #[must_use]
    pub const fn inventory_capacity(&self) -> &Knowledge<u8> {
        &self.inventory_capacity
    }

    /// Returns projected economy values.
    #[must_use]
    pub const fn economy(&self) -> &Economy {
        &self.economy
    }

    pub(crate) fn clear_location(&mut self) -> bool {
        if matches!(self.location, Location::Unknown) {
            return false;
        }

        self.location = Location::Unknown;
        true
    }

    pub(crate) fn observe_location(
        &mut self,
        location: &protocol::Location,
        revision: Revision,
    ) -> bool {
        let observed = Location::from_protocol(location, revision);
        let changed = !self.location.same_value(&observed);

        self.location = observed;
        changed
    }

    pub(crate) fn observe_movement(
        &mut self,
        movement: &action::Movement,
        revision: Revision,
    ) -> bool {
        let Some(position) = movement.origin().step(movement.direction()) else {
            return false;
        };
        let observed = Location::ClientReported {
            position,
            viewport: self.location.viewport(),
            evidence: ClientEvidence::Movement {
                origin: movement.origin(),
                last_walk: movement.last_walk(),
            },
            revision,
        };
        let location_changed = !self.location.same_value(&observed);

        self.location = observed;
        location_changed | self.observe_direction(movement.direction(), revision)
    }

    pub(crate) fn observe_facing(&mut self, facing: action::Facing, revision: Revision) -> bool {
        self.observe_direction(facing.direction(), revision)
    }

    pub(crate) fn observe_obstruction(
        &mut self,
        obstruction: action::Obstruction,
        revision: Revision,
    ) -> bool {
        let observed = Location::ClientReported {
            position: obstruction.origin(),
            viewport: self.location.viewport(),
            evidence: ClientEvidence::Obstruction {
                direction: obstruction.direction(),
            },
            revision,
        };
        let location_changed = !self.location.same_value(&observed);

        self.location = observed;
        location_changed | self.observe_direction(obstruction.direction(), revision)
    }

    pub(crate) fn observe_status(&mut self, status: &protocol::Status, revision: Revision) -> bool {
        let mut changed = false;

        if let Some(resources) = status.resources() {
            changed |= self.resources.vita.observe_current(
                resources.vita(),
                revision,
                Source::PlayerStatus,
            );
            changed |= self.resources.mana.observe_current(
                resources.mana(),
                revision,
                Source::PlayerStatus,
            );
        }

        if let Some(full) = status.full() {
            changed |= self.resources.vita.observe_maximum(
                full.max_vita(),
                revision,
                Source::PlayerStatus,
            );
            changed |= self.resources.mana.observe_maximum(
                full.max_mana(),
                revision,
                Source::PlayerStatus,
            );

            if self.level.value() != Some(&full.level()) {
                self.level = Knowledge::observed(full.level(), revision, Source::PlayerStatus);
                changed = true;
            }

            if self.inventory_capacity.value() != Some(&full.max_inventory()) {
                self.inventory_capacity =
                    Knowledge::observed(full.max_inventory(), revision, Source::PlayerStatus);
                changed = true;
            }
        }

        if let Some(economy) = status.economy() {
            if self.economy.experience.value() != Some(&economy.experience()) {
                self.economy.experience =
                    Knowledge::observed(economy.experience(), revision, Source::PlayerStatus);
                changed = true;
            }
            if self.economy.money.value() != Some(&economy.money()) {
                self.economy.money =
                    Knowledge::observed(economy.money(), revision, Source::PlayerStatus);
                changed = true;
            }
            if self.economy.experience_percent.value() != Some(&economy.experience_percent()) {
                self.economy.experience_percent = Knowledge::observed(
                    economy.experience_percent(),
                    revision,
                    Source::PlayerStatus,
                );
                changed = true;
            }
        }

        changed
    }

    pub(crate) fn seed_resources(
        &mut self,
        resources: resource::Resources,
        revision: Revision,
    ) -> bool {
        self.resources.vita.seed(resources.vita(), revision)
            | self.resources.mana.seed(resources.mana(), revision)
    }

    fn observe_direction(&mut self, direction: Direction, revision: Revision) -> bool {
        if self.facing.value() == Some(&direction) {
            return false;
        }

        self.facing = Knowledge::observed(direction, revision, Source::ClientAction);
        true
    }
}
