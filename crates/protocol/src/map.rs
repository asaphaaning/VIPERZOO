//! Map context and streamed tile rectangles.

use std::num::{NonZeroU8, NonZeroU16};

use serde::Serialize;

use crate::packet::HasOpaqueTail;
use crate::primitive::{MapId, Opaque, Position};

/// Non-zero full-map dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Dimensions {
    width: NonZeroU16,
    height: NonZeroU16,
}

impl Dimensions {
    /// Creates dimensions, or `None` when either extent is zero.
    ///
    /// A map the client can render has both extents, so a zero here means the
    /// source has no map rather than a map of no size.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Option<Self> {
        Some(Self {
            width: NonZeroU16::new(width)?,
            height: NonZeroU16::new(height)?,
        })
    }

    /// Returns the full map width.
    #[must_use]
    pub const fn width(self) -> u16 {
        self.width.get()
    }

    /// Returns the full map height.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.height.get()
    }
}

/// Which map is active, and how large it is.
///
/// Identifier and dimensions travel together because neither source of map
/// identity offers one without the other: the server's [`Context`] carries
/// both, and the client stores and re-validates both before it will reload a
/// map. That matters for memory-derived identity, where a bare identifier is
/// unverifiable — any two bytes can equal a map number — while the pair can be
/// checked against a map that is actually known.
///
/// ```
/// use viperzoo_protocol::map::{Dimensions, Identity};
/// use viperzoo_protocol::primitive::MapId;
///
/// let grove = Identity::new(MapId::new(0x0482), Dimensions::new(60, 60).unwrap());
///
/// assert_eq!(grove.id().value(), 0x0482);
/// assert_eq!(grove.dimensions().width(), 60);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Identity {
    id: MapId,
    dimensions: Dimensions,
}

impl Identity {
    /// Joins a map identifier to the dimensions observed alongside it.
    #[must_use]
    pub const fn new(id: MapId, dimensions: Dimensions) -> Self {
        Self { id, dimensions }
    }

    /// Returns the map identifier.
    #[must_use]
    pub const fn id(self) -> MapId {
        self.id
    }

    /// Returns the full map dimensions.
    #[must_use]
    pub const fn dimensions(self) -> Dimensions {
        self.dimensions
    }
}

/// Identity and environment of the active map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Context {
    id: MapId,
    dimensions: Dimensions,
    weather: u8,
    realm: u8,
    title: String,
    light: u16,
    opaque_tail: Opaque,
}

impl Context {
    pub(crate) fn new(
        id: MapId,
        dimensions: Dimensions,
        weather: u8,
        realm: u8,
        title: String,
        light: u16,
        opaque_tail: Opaque,
    ) -> Self {
        Self {
            id,
            dimensions,
            weather,
            realm,
            title,
            light,
            opaque_tail,
        }
    }

    /// Returns the map identifier.
    #[must_use]
    pub const fn id(&self) -> MapId {
        self.id
    }

    /// Returns the full map dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    /// Returns the identity this context establishes.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        Identity::new(self.id, self.dimensions)
    }

    /// Returns the display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the map weather value.
    #[must_use]
    pub const fn weather(&self) -> u8 {
        self.weather
    }

    /// Returns the realm flag.
    #[must_use]
    pub const fn realm(&self) -> u8 {
        self.realm
    }

    /// Returns the map light value.
    #[must_use]
    pub const fn light(&self) -> u16 {
        self.light
    }
}

impl HasOpaqueTail for Context {
    fn opaque_tail(&self) -> &Opaque {
        &self.opaque_tail
    }
}

/// Non-zero dimensions of one streamed map rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RegionSize {
    width: NonZeroU8,
    height: NonZeroU8,
}

impl RegionSize {
    pub(crate) fn new(width: u8, height: u8) -> Option<Self> {
        let size = Self {
            width: NonZeroU8::new(width)?,
            height: NonZeroU8::new(height)?,
        };

        (size.tile_count() <= 323).then_some(size)
    }

    /// Returns the rectangle width.
    #[must_use]
    pub const fn width(self) -> u8 {
        self.width.get()
    }

    /// Returns the rectangle height.
    #[must_use]
    pub const fn height(self) -> u8 {
        self.height.get()
    }

    /// Returns the number of row-major cells.
    #[must_use]
    pub const fn tile_count(self) -> usize {
        self.width.get() as usize * self.height.get() as usize
    }
}

/// One static map tile record.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Tile {
    ground_id: u16,
    pass_value: u16,
    object_id: u16,
}

impl Tile {
    pub(crate) const fn new(ground_id: u16, pass_value: u16, object_id: u16) -> Self {
        Self {
            ground_id,
            pass_value,
            object_id,
        }
    }

    /// Returns the ground graphic identifier.
    #[must_use]
    pub const fn ground_id(self) -> u16 {
        self.ground_id
    }

    /// Returns the authoritative static pass value.
    #[must_use]
    pub const fn pass_value(self) -> u16 {
        self.pass_value
    }

    /// Returns the static fixture/object identifier.
    #[must_use]
    pub const fn object_id(self) -> u16 {
        self.object_id
    }

    /// Returns whether the server map marks this tile impassable.
    #[must_use]
    pub const fn blocks_movement(self) -> bool {
        self.pass_value != 0
    }
}

/// A row-major static map rectangle or scrolling strip.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Region {
    origin: Position,
    size: RegionSize,
    tiles: Vec<Tile>,
    opaque_tail: Opaque,
}

impl Region {
    pub(crate) fn new(
        origin: Position,
        size: RegionSize,
        tiles: Vec<Tile>,
        opaque_tail: Opaque,
    ) -> Option<Self> {
        (tiles.len() == size.tile_count()).then_some(Self {
            origin,
            size,
            tiles,
            opaque_tail,
        })
    }

    /// Returns the global rectangle origin.
    #[must_use]
    pub const fn origin(&self) -> Position {
        self.origin
    }

    /// Returns the rectangle dimensions.
    #[must_use]
    pub const fn size(&self) -> RegionSize {
        self.size
    }

    /// Borrows row-major tile records.
    #[must_use]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
}

impl HasOpaqueTail for Region {
    fn opaque_tail(&self) -> &Opaque {
        &self.opaque_tail
    }
}
