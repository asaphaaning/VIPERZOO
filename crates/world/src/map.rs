//! Keep map identity and streamed tile coverage coherent across attachment.
//!
//! A late attachment can observe tile rectangles before it receives a map
//! context. [`State`] therefore represents that real uncertainty instead of
//! pairing old coordinates with a guessed map identifier. Once context arrives,
//! coverage and identity become one [`Epoch`]; a later identity change starts a
//! fresh epoch so tiles from different maps cannot be silently merged.
//!
//! Navigation can use unidentified coverage only as a bounded local area. It
//! may use identified coverage together with decoded map dimensions.

use std::collections::BTreeMap;

use serde::Serialize;
use viperzoo_protocol::{map as protocol, primitive::Position};

/// A monotonically increasing map identity epoch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Epoch(u64);

impl Epoch {
    /// Returns the following map epoch.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Returns the numeric epoch.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Map context without packet-boundary workspace bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Context {
    id: viperzoo_protocol::primitive::MapId,
    dimensions: protocol::Dimensions,
    weather: u8,
    realm: u8,
    title: String,
    light: u16,
}

impl Context {
    fn from_protocol(context: &protocol::Context) -> Self {
        Self {
            id: context.id(),
            dimensions: context.dimensions(),
            weather: context.weather(),
            realm: context.realm(),
            title: context.title().to_owned(),
            light: context.light(),
        }
    }

    /// Returns the stable map identifier.
    #[must_use]
    pub const fn id(&self) -> viperzoo_protocol::primitive::MapId {
        self.id
    }

    /// Returns the full map dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> protocol::Dimensions {
        self.dimensions
    }

    /// Returns the display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the weather value.
    #[must_use]
    pub const fn weather(&self) -> u8 {
        self.weather
    }

    /// Returns the realm flag.
    #[must_use]
    pub const fn realm(&self) -> u8 {
        self.realm
    }

    /// Returns the light value.
    #[must_use]
    pub const fn light(&self) -> u16 {
        self.light
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.id == other.id && self.dimensions == other.dimensions
    }
}

/// One globally positioned static tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Tile {
    position: Position,
    ground_id: u16,
    pass_value: u16,
    object_id: u16,
}

impl Tile {
    fn from_protocol(position: Position, tile: protocol::Tile) -> Self {
        Self {
            position,
            ground_id: tile.ground_id(),
            pass_value: tile.pass_value(),
            object_id: tile.object_id(),
        }
    }

    /// Returns the global coordinate.
    #[must_use]
    pub const fn position(self) -> Position {
        self.position
    }

    /// Returns the ground graphic identifier.
    #[must_use]
    pub const fn ground_id(self) -> u16 {
        self.ground_id
    }

    /// Returns the server pass value.
    #[must_use]
    pub const fn pass_value(self) -> u16 {
        self.pass_value
    }

    /// Returns the static fixture identifier.
    #[must_use]
    pub const fn object_id(self) -> u16 {
        self.object_id
    }

    /// Returns whether server map data marks the tile impassable.
    #[must_use]
    pub const fn blocks_movement(self) -> bool {
        self.pass_value != 0
    }
}

/// Map knowledge valid for both warm and cold attachment.
#[derive(Debug)]
pub(crate) enum State {
    /// Tile coverage exists without an observed map identity.
    Unidentified {
        epoch: Epoch,
        tiles: BTreeMap<Position, Tile>,
    },
    /// Map identity and tile coverage are joined in one epoch.
    Identified {
        epoch: Epoch,
        context: Context,
        tiles: BTreeMap<Position, Tile>,
    },
}

impl Default for State {
    fn default() -> Self {
        Self::Unidentified {
            epoch: Epoch::default(),
            tiles: BTreeMap::new(),
        }
    }
}

impl State {
    pub(crate) fn observe_context(&mut self, observed: &protocol::Context) -> Transition {
        let observed = Context::from_protocol(observed);
        let previous = std::mem::take(self);

        match previous {
            Self::Unidentified { epoch, tiles } => {
                *self = Self::Identified {
                    epoch: epoch.next(),
                    context: observed,
                    tiles,
                };

                Transition::Identified
            }
            Self::Identified {
                epoch,
                context,
                tiles,
            } if context.same_identity(&observed) => {
                let changed = context != observed;

                *self = Self::Identified {
                    epoch,
                    context: observed,
                    tiles,
                };

                if changed {
                    Transition::Updated
                } else {
                    Transition::Unchanged
                }
            }
            Self::Identified { epoch, .. } => {
                *self = Self::Identified {
                    epoch: epoch.next(),
                    context: observed,
                    tiles: BTreeMap::new(),
                };

                Transition::Changed
            }
        }
    }

    /// Incorporates one streamed static-tile rectangle into current map coverage.
    ///
    /// A [`protocol::Region`] is a row-major rectangle whose tile records are
    /// relative to its origin. This method gives every record its absolute
    /// [`Position`] and updates only those coordinates. Existing coverage
    /// outside the rectangle remains available, so initial map data, scrolling
    /// strips, and refreshes accumulate into one view of the active map.
    ///
    /// Overlap is replacement, not a conflict. A later region is the newer
    /// server description of each coordinate it contains, including its ground
    /// ID, pass value, and static fixture ID. Replaying an identical region is
    /// therefore a no-op; refreshing one coordinate with a different tile
    /// replaces exactly that coordinate:
    ///
    /// ```text
    /// Existing coverage                    Incoming region, origin (11, 10)
    /// (10, 10) = A  (11, 10) = B            (11, 10) = C  (12, 10) = D
    ///
    /// Resulting coverage
    /// (10, 10) = A  (11, 10) = C  (12, 10) = D
    /// ```
    ///
    /// Region data does not establish or change map identity. It can be
    /// accepted while the attachment is [`State::Unidentified`], and it is
    /// scoped to a new map only when [`State::observe_context`] records an
    /// identity change and clears the old coverage. The returned value says
    /// whether the projected coverage changed, not whether the packet was
    /// merely observed.
    pub(crate) fn merge(&mut self, region: &protocol::Region) -> bool {
        let tiles = match self {
            Self::Unidentified { tiles, .. } | Self::Identified { tiles, .. } => tiles,
        };
        let width = usize::from(region.size().width());
        let mut changed = false;

        for (index, tile) in region.tiles().iter().copied().enumerate() {
            let horizontal = u16::try_from(index % width).ok();
            let vertical = u16::try_from(index / width).ok();
            let Some(position) = horizontal.zip(vertical).and_then(|(horizontal, vertical)| {
                Some(Position::new(
                    region.origin().x().value().checked_add(horizontal)?,
                    region.origin().y().value().checked_add(vertical)?,
                ))
            }) else {
                continue;
            };
            let projected = Tile::from_protocol(position, tile);

            changed |= tiles.insert(position, projected) != Some(projected);
        }

        changed
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        match self {
            Self::Unidentified { epoch, tiles } => Snapshot::Unidentified {
                epoch: *epoch,
                tiles: tiles.values().copied().collect(),
            },
            Self::Identified {
                epoch,
                context,
                tiles,
            } => Snapshot::Identified {
                epoch: *epoch,
                context: context.clone(),
                tiles: tiles.values().copied().collect(),
            },
        }
    }
}

/// Effect of observing a map context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Transition {
    Unchanged,
    Identified,
    Updated,
    Changed,
}

impl Transition {
    pub(crate) const fn changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    pub(crate) const fn invalidates_scoped_state(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Immutable map projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Snapshot {
    /// Static coverage is available, but no map identity was observed.
    Unidentified {
        /// Current provisional epoch.
        epoch: Epoch,
        /// Known coordinate-indexed static tiles.
        tiles: Vec<Tile>,
    },
    /// Identity and coverage belong to one map epoch.
    Identified {
        /// Current map epoch.
        epoch: Epoch,
        /// Active map identity and environment.
        context: Context,
        /// Known coordinate-indexed static tiles.
        tiles: Vec<Tile>,
    },
}

impl Snapshot {
    /// Returns the map epoch.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        match self {
            Self::Unidentified { epoch, .. } | Self::Identified { epoch, .. } => *epoch,
        }
    }

    /// Returns identified context when observed.
    #[must_use]
    pub const fn context(&self) -> Option<&Context> {
        match self {
            Self::Unidentified { .. } => None,
            Self::Identified { context, .. } => Some(context),
        }
    }

    /// Borrows all known static tiles.
    #[must_use]
    pub fn tiles(&self) -> &[Tile] {
        match self {
            Self::Unidentified { tiles, .. } | Self::Identified { tiles, .. } => tiles,
        }
    }

    /// Finds one known tile.
    #[must_use]
    pub fn tile(&self, position: Position) -> Option<Tile> {
        self.tiles()
            .binary_search_by_key(&position, |tile| tile.position())
            .ok()
            .map(|index| self.tiles()[index])
    }

    /// Returns the number of server-blocked tiles.
    #[must_use]
    pub fn blocked_tile_count(&self) -> usize {
        self.tiles()
            .iter()
            .filter(|tile| tile.blocks_movement())
            .count()
    }
}
