//! Keep map identity and streamed tile coverage coherent across attachment.
//!
//! A late attachment can observe tile rectangles before it receives a map
//! context. [`State`] therefore represents that real uncertainty instead of
//! pairing old coordinates with a guessed map identifier. The first observed
//! context starts a fresh [`Epoch`] because the client may have crossed a map
//! boundary after attachment; later identity changes do the same. Tiles from
//! different maps therefore cannot be silently merged.
//!
//! Both advances move the same counter, so the counter alone cannot say what
//! happened. [`Epoch::origin`] carries that distinction to callers holding
//! map-scoped work:
//!
//! ```text
//! Unidentified ──first context──► Identified   Origin::Established
//!                                     │        (we learned where we were)
//!                                     │
//!                                     └─new identity─► Identified
//!                                                      Origin::Crossed
//!                                                      (we are elsewhere)
//! ```
//!
//! Navigation can use unidentified coverage only as a bounded local area. It
//! may use identified coverage together with decoded map dimensions.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use viperzoo_protocol::{map as protocol, primitive::Position};

/// How a map [`Epoch`] came to be.
///
/// Map-scoped work is planned against one epoch. When the epoch advances, this
/// is what separates "we finally learned which map we were already on" from
/// "we are standing somewhere else now". Both advance the counter, and they
/// demand opposite recoveries, so the counter alone cannot decide.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// No map identity has been observed since attachment.
    ///
    /// Coverage is provisional: it may serve as a bounded local area, but no
    /// [`Context`] justifies a map-wide claim.
    #[default]
    Attachment,
    /// Identity was first established for previously unidentified coverage.
    ///
    /// The player did not necessarily move. A warm attachment reaches this
    /// origin on its first observed context, so a destination planned in the
    /// previous epoch is stale rather than wrong about the world.
    Established,
    /// Identity replaced a different identified map.
    ///
    /// The player genuinely crossed a boundary, so a coordinate planned in the
    /// previous epoch names a different place and must not be reused.
    Crossed,
}

impl Origin {
    /// Every [`Origin`], in declaration order.
    pub const VARIANTS: [Self; 3] = [Self::Attachment, Self::Established, Self::Crossed];
}

/// A monotonically increasing map identity epoch and the event that began it.
///
/// The counter orders epochs; [`Epoch::origin`] explains the most recent
/// advance. Both matter: comparing counters detects that map-scoped work is
/// stale, and the [`Origin`] decides whether that work can be replanned or must
/// be abandoned.
///
/// ```
/// use viperzoo_world::map::{Epoch, Origin};
///
/// let attached = Epoch::ATTACHMENT;
/// assert_eq!(attached.origin(), Origin::Attachment);
///
/// // A warm attachment learning where it already was.
/// let identified = attached.established();
/// assert_eq!(identified.value(), 1);
/// assert_eq!(identified.origin(), Origin::Established);
///
/// // Walking through a portal into a different map.
/// let elsewhere = identified.crossed();
/// assert_eq!(elsewhere.value(), 2);
/// assert_eq!(elsewhere.origin(), Origin::Crossed);
///
/// // Both advanced the counter; only the origin says which happened.
/// assert_ne!(identified.origin(), elsewhere.origin());
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Epoch {
    value: u64,
    origin: Origin,
}

impl Epoch {
    /// The epoch of an attachment that has not yet observed map identity.
    pub const ATTACHMENT: Self = Self {
        value: 0,
        origin: Origin::Attachment,
    };

    /// Returns the epoch beginning when identity first arrives for coverage
    /// that had none, yielding [`Origin::Established`].
    #[must_use]
    pub const fn established(self) -> Self {
        Self {
            value: self.value + 1,
            origin: Origin::Established,
        }
    }

    /// Returns the epoch beginning when identity replaces a different known
    /// map, yielding [`Origin::Crossed`].
    #[must_use]
    pub const fn crossed(self) -> Self {
        Self {
            value: self.value + 1,
            origin: Origin::Crossed,
        }
    }

    /// Returns the numeric epoch.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    /// Returns how this epoch began.
    #[must_use]
    pub const fn origin(self) -> Origin {
        self.origin
    }
}

/// Serializes as the bare counter, so projections keep their numeric shape.
impl Serialize for Epoch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.value)
    }
}

/// Surroundings the server describes alongside map identity.
///
/// Only a `0x15` context carries these. Client memory publishes which map is
/// active without describing it, so [`Context::environment`] is absent for a
/// memory-derived identity rather than defaulted to zeroes that would read as
/// real observations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Environment {
    weather: u8,
    realm: u8,
    light: u16,
}

impl Environment {
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
}

/// Map context without packet-boundary workspace bytes.
///
/// Identity is always present; [`Environment`] is present only when the server
/// described the map. The two sources differ in what they can know, not in how
/// much they are trusted for identity:
///
/// ```text
/// 0x15 context   ──►  Identity + Environment
/// client memory  ──►  Identity
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Context {
    #[serde(flatten)]
    identity: protocol::Identity,
    /// Both sources can name the map, so the title is its own field rather than
    /// part of [`Environment`]: a warm attachment reads it from the client's own
    /// model without learning anything about the weather.
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// Flattened so a described map keeps its established projection shape and
    /// an undescribed one simply omits those keys, rather than reporting
    /// zeroes that would read as real weather, realm, and light observations.
    #[serde(flatten)]
    environment: Option<Environment>,
}

impl Context {
    fn from_protocol(context: &protocol::Context) -> Self {
        Self {
            identity: context.identity(),
            title: Some(context.title().to_owned()),
            environment: Some(Environment {
                weather: context.weather(),
                realm: context.realm(),
                light: context.light(),
            }),
        }
    }

    /// Builds context from what a warm attachment can read out of the client.
    ///
    /// The client keeps identity and title in different places — identity on
    /// the map model, the title on the object published beside the resource
    /// model — and neither carries the weather. `title` is therefore optional
    /// independently of identity, and [`Environment`] stays absent entirely.
    #[must_use]
    pub const fn from_identity(identity: protocol::Identity, title: Option<String>) -> Self {
        Self {
            identity,
            title,
            environment: None,
        }
    }

    /// Returns the stable map identifier.
    #[must_use]
    pub const fn id(&self) -> viperzoo_protocol::primitive::MapId {
        self.identity.id()
    }

    /// Returns the full map dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> protocol::Dimensions {
        self.identity.dimensions()
    }

    /// Returns the identity this context establishes.
    #[must_use]
    pub const fn identity(&self) -> protocol::Identity {
        self.identity
    }

    /// Returns the server-described surroundings, when the server described them.
    #[must_use]
    pub const fn environment(&self) -> Option<&Environment> {
        self.environment.as_ref()
    }

    /// Returns the display title, from whichever source supplied one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
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
            epoch: Epoch::ATTACHMENT,
            tiles: BTreeMap::new(),
        }
    }
}

impl State {
    pub(crate) fn observe_context(&mut self, observed: &protocol::Context) -> Transition {
        let observed = Context::from_protocol(observed);
        let previous = std::mem::take(self);

        match previous {
            Self::Unidentified { epoch, .. } => {
                *self = Self::Identified {
                    epoch: epoch.established(),
                    context: observed,
                    tiles: BTreeMap::new(),
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
                    epoch: epoch.crossed(),
                    context: observed,
                    tiles: BTreeMap::new(),
                };

                Transition::Changed
            }
        }
    }

    /// Establishes identity read from the client's own model.
    ///
    /// A warm attachment joins a client that has already been told where it is,
    /// so the `0x15` context that would answer the question has long passed.
    /// The client keeps the answer to render, and this accepts that reading —
    /// but only to fill a gap. A context the server actually sent describes the
    /// map as well as naming it, so it always wins:
    ///
    /// ```text
    /// Unidentified            ──► Identified (Established)
    /// Identified, same map    ──► unchanged, environment preserved
    /// Identified, other map   ──► unchanged; the server's word stands
    /// ```
    pub(crate) fn observe_identity(
        &mut self,
        observed: protocol::Identity,
        title: Option<String>,
    ) -> Transition {
        match self {
            Self::Unidentified { epoch, .. } => {
                *self = Self::Identified {
                    epoch: epoch.established(),
                    context: Context::from_identity(observed, title),
                    tiles: BTreeMap::new(),
                };

                Transition::Identified
            }
            Self::Identified { .. } => Transition::Unchanged,
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
    /// accepted while the attachment is [`State::Unidentified`]. Because that
    /// provisional coverage has no identity, [`State::observe_context`] starts
    /// a fresh epoch and clears it when the first context arrives. The returned
    /// value says whether the projected coverage changed, not whether the
    /// packet was merely observed.
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
        matches!(self, Self::Identified | Self::Changed)
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
