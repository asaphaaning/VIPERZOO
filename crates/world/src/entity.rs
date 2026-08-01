//! Projected visible entities and spatial queries.

use serde::Serialize;
use viperzoo_protocol::{
    direction::Direction,
    entity as protocol,
    primitive::{EntityId, Position},
};

use crate::revision::Revision;

/// Latest server-authored VITA observation for a visible entity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Vitality {
    percent: u8,
    last_amount: i32,
    hit_type: u8,
    revision: Revision,
}

impl Vitality {
    fn from_protocol(value: &viperzoo_protocol::combat::Vitality, revision: Revision) -> Self {
        Self {
            percent: value.percent(),
            last_amount: value.amount(),
            hit_type: value.hit_type(),
            revision,
        }
    }

    /// Returns the server's current clamped health percentage.
    #[must_use]
    pub const fn percent(self) -> u8 {
        self.percent
    }

    /// Returns the most recently reported signed damage/healing amount.
    #[must_use]
    pub const fn last_amount(self) -> i32 {
        self.last_amount
    }

    /// Returns the revision of this health observation.
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }
}

/// Appearance knowledge for an entity first observed through movement.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Appearance {
    /// A movement arrived before an appearance record.
    #[default]
    Unknown,
    /// A typed appearance record established visual identity and occupancy.
    Observed {
        /// Visual entity kind.
        kind: protocol::Kind,
        /// Look identifier.
        look_id: u16,
        /// Look color.
        look_color: u8,
        /// Unresolved appearance byte retained as evidence.
        unknown: u8,
        /// Current animation descriptors.
        animations: Vec<protocol::Animation>,
    },
}

impl Appearance {
    fn from_protocol(appearance: &protocol::Appearance) -> Self {
        Self::Observed {
            kind: appearance.kind(),
            look_id: appearance.look_id(),
            look_color: appearance.look_color(),
            unknown: appearance.unknown(),
            animations: appearance.animations().to_vec(),
        }
    }

    /// Returns occupancy knowledge implied by the visual record type.
    #[must_use]
    pub const fn occupancy(&self) -> protocol::Occupancy {
        match self {
            Self::Unknown => protocol::Occupancy::Unknown,
            Self::Observed { kind, .. } => kind.occupancy(),
        }
    }

    /// Returns whether this is a known floor item.
    #[must_use]
    pub const fn is_floor_item(&self) -> bool {
        matches!(
            self,
            Self::Observed {
                kind: protocol::Kind::FloorItem,
                ..
            }
        )
    }
}

/// One visible entity in the active map epoch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct State {
    id: EntityId,
    position: Position,
    direction: Direction,
    appearance: Appearance,
    vitality: Option<Vitality>,
    revision: Revision,
}

impl State {
    pub(crate) fn appeared(
        appearance: &protocol::Appearance,
        previous: Option<&Self>,
        revision: Revision,
    ) -> Self {
        Self {
            id: appearance.id(),
            position: appearance.position(),
            direction: appearance.direction(),
            appearance: Appearance::from_protocol(appearance),
            vitality: previous.and_then(|entity| entity.vitality),
            revision,
        }
    }

    pub(crate) fn moved(
        previous: Option<&Self>,
        movement: &protocol::Movement,
        revision: Revision,
    ) -> Option<Self> {
        Some(Self {
            id: movement.id(),
            position: movement.destination()?,
            direction: movement.direction(),
            appearance: previous
                .map_or_else(Appearance::default, |entity| entity.appearance.clone()),
            vitality: previous.and_then(|entity| entity.vitality),
            revision,
        })
    }

    pub(crate) fn observe_vitality(
        &mut self,
        value: &viperzoo_protocol::combat::Vitality,
        revision: Revision,
    ) -> bool {
        let next = Vitality::from_protocol(value, revision);
        let changed = self.vitality != Some(next);

        self.vitality = Some(next);
        self.revision = revision;
        changed
    }

    /// Returns the stable runtime identifier.
    #[must_use]
    pub const fn id(&self) -> EntityId {
        self.id
    }

    /// Returns the current map tile.
    #[must_use]
    pub const fn position(&self) -> Position {
        self.position
    }

    /// Returns current facing/movement direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns appearance knowledge.
    #[must_use]
    pub const fn appearance(&self) -> &Appearance {
        &self.appearance
    }

    /// Returns the latest current VITA observation, when one has arrived.
    #[must_use]
    pub const fn vitality(&self) -> Option<Vitality> {
        self.vitality
    }

    /// Returns the revision of the latest entity observation.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub(crate) fn same_value(&self, other: &Self) -> bool {
        self.id == other.id
            && self.position == other.position
            && self.direction == other.direction
            && self.appearance == other.appearance
            && self.vitality == other.vitality
    }
}
