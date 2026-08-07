//! Decode `SObj.tbl` records into directional fixture knowledge.
//!
//! A live map tile gives a one-based object ID, not the record's contents.
//! `SObj.tbl` supplies those contents. [`Catalog`] keeps this static data
//! separate from the live world, then uses the object ID as the join key when
//! navigation evaluates a current tile.
//!
//! Each record leads with its fixed prefix and ends with a variable tile list:
//!
//! ```text
//! header │ u32 record_count │ 2 bytes belonging to no record
//! ───────┼──────────────────┴────────────────────────────────
//! record │ 5 metadata │ 1 collision │ 1 tile_count │ u16le * tile_count
//! ```
//!
//! The prefix order matters. Reading the tile list first also tiles the file
//! exactly — the records are a rotation of each other — but silently attributes
//! every tile list to the following object. Collision masks land on the same
//! offsets either way, which is why live collision evidence cannot distinguish
//! the two readings.
//!
//! The collision byte answers a narrower question than the server pass value:
//! whether this fixture blocks entry from a particular cardinal direction. A
//! missing record or unknown metadata does not become a guessed obstruction;
//! only the validated directional bits contribute collision evidence.

use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
};

use thiserror::Error;
use tracing::instrument;
use viperzoo_protocol::direction::Direction;

use crate::archive;

const UP: u8 = 0x01;
const DOWN: u8 = 0x02;
const RIGHT: u8 = 0x04;
const LEFT: u8 = 0x08;

/// `u32 record_count` followed by two bytes that belong to no record.
const HEADER: usize = 6;
/// Opaque leading bytes of a record, before its collision mask.
const METADATA: usize = 5;
/// Fixed part of a record: [`METADATA`], the collision mask, then a tile count.
const PREFIX: usize = METADATA + 2;

/// Directional entry restrictions declared by a static fixture.
///
/// The low nibble is a fully observed vocabulary: every one of the four
/// cardinal bits appears across the live client's table, in all sixteen
/// combinations. The upper nibble is not. Exactly one record — object 2068 —
/// carries `0xF1`, so this type preserves the whole byte rather than masking
/// to the bits it understands. Discarding them would erase the only evidence
/// that the field has more to say.
///
/// ```
/// use viperzoo_assets::Collision;
/// use viperzoo_protocol::direction::Direction;
///
/// let rock = Collision::new(0x0F);
/// assert!(Direction::VARIANTS.iter().all(|way| rock.blocks(*way)));
/// assert_eq!(rock.unknown_bits(), 0);
///
/// let ledge = Collision::new(0x01);
/// assert!(ledge.blocks(Direction::Up));
/// assert!(!ledge.blocks(Direction::Down));
///
/// // The live outlier keeps its unexplained bits instead of decoding as 0x01.
/// let outlier = Collision::new(0xF1);
/// assert!(outlier.blocks(Direction::Up));
/// assert_eq!(outlier.unknown_bits(), 0xF0);
/// assert_ne!(outlier, ledge);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Collision(u8);

impl Collision {
    /// A fixture that restricts no direction.
    pub const PASSABLE: Self = Self(0);

    /// Creates a mask from one raw `SObj.tbl` collision byte.
    #[must_use]
    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    /// Returns whether entering the fixture from `direction` is blocked.
    #[must_use]
    pub const fn blocks(self, direction: Direction) -> bool {
        let bit = match direction {
            Direction::Up => UP,
            Direction::Right => RIGHT,
            Direction::Down => DOWN,
            Direction::Left => LEFT,
        };

        self.0 & bit != 0
    }

    /// Returns whether the fixture restricts no cardinal direction.
    #[must_use]
    pub const fn is_passable(self) -> bool {
        self.0 & (UP | DOWN | RIGHT | LEFT) == 0
    }

    /// Returns the bits outside the four understood cardinal flags.
    ///
    /// A nonzero result marks a record whose meaning is not fully decoded.
    #[must_use]
    pub const fn unknown_bits(self) -> u8 {
        self.0 & !(UP | DOWN | RIGHT | LEFT)
    }

    /// Returns the undecoded byte exactly as the client stores it.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// The fixed `SObj.tbl` record fields whose meaning is not yet established.
///
/// These five bytes were previously carried as one opaque block. They are not
/// opaque: the leading four are a little-endian `u32` in which all-ones encodes
/// absence, holding a sentinel in 19,056 of the live client's 19,551 records
/// and otherwise a value in `1..=105`. No record disagrees between the first
/// byte and the following three, and no present value exceeds `0xFFFF`.
///
/// The shape is therefore certain and the semantics are not, so the accessors
/// describe structure only. [`Metadata::reference`] does not claim to know what
/// it indexes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    reference: Option<u32>,
    subtype: u8,
}

impl Metadata {
    /// Encodes an absent [`Metadata::reference`].
    pub const ABSENT: u32 = u32::MAX;

    /// Decodes the fixed metadata bytes of one record.
    #[must_use]
    pub const fn new(reference: Option<u32>, subtype: u8) -> Self {
        Self { reference, subtype }
    }

    fn decode(bytes: [u8; METADATA]) -> Self {
        let reference = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

        Self {
            reference: (reference != Self::ABSENT).then_some(reference),
            subtype: bytes[4],
        }
    }

    /// Returns the small unresolved index, absent when the client stored the
    /// all-ones sentinel.
    ///
    /// What this refers to is undetermined; only its optionality and range are
    /// evidence-backed.
    #[must_use]
    pub const fn reference(self) -> Option<u32> {
        self.reference
    }

    /// Returns the trailing classification byte, observed only in `0..=6`.
    #[must_use]
    pub const fn subtype(self) -> u8 {
        self.subtype
    }
}

/// One static fixture decoded from a one-based `SObj.tbl` record.
///
/// A live map tile refers to a fixture only by its [`Fixture::id`]. This value
/// provides the client-version-specific details for that identifier: its visual
/// tile IDs, unresolved [`Metadata`], and a [`Collision`] mask. It is not a
/// live world object and does not carry a position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fixture {
    id: u16,
    tile_ids: Vec<u16>,
    metadata: Metadata,
    collision: Collision,
}

impl Fixture {
    /// Creates one decoded or synthetic static fixture.
    #[must_use]
    pub fn new(id: u16, tile_ids: Vec<u16>, metadata: Metadata, collision: Collision) -> Self {
        Self {
            id,
            tile_ids,
            metadata,
            collision,
        }
    }

    /// Returns the one-based object identifier used by map tiles.
    #[must_use]
    pub const fn id(&self) -> u16 {
        self.id
    }

    /// Borrows visual tile identifiers in this static fixture.
    #[must_use]
    pub fn tile_ids(&self) -> &[u16] {
        &self.tile_ids
    }

    /// Returns the record's structurally decoded but semantically unresolved
    /// fields.
    #[must_use]
    pub const fn metadata(&self) -> Metadata {
        self.metadata
    }

    /// Returns the directional collision mask.
    #[must_use]
    pub const fn collision(&self) -> Collision {
        self.collision
    }

    /// Returns whether entering this fixture in `direction` is blocked.
    #[must_use]
    pub const fn blocks(&self, direction: Direction) -> bool {
        self.collision.blocks(direction)
    }
}

/// Decoded static fixtures indexed for a live map-tile join.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Catalog {
    fixtures: BTreeMap<u16, Fixture>,
    source: PathBuf,
}

impl Catalog {
    /// Creates a catalog from decoded static fixtures.
    #[must_use]
    pub fn new(fixtures: impl IntoIterator<Item = Fixture>, source: PathBuf) -> Self {
        Self {
            fixtures: fixtures
                .into_iter()
                .map(|fixture| (fixture.id, fixture))
                .collect(),
            source,
        }
    }

    /// Returns the static fixture for one one-based live map object identifier.
    #[must_use]
    pub fn fixture(&self, id: u16) -> Option<&Fixture> {
        self.fixtures.get(&id)
    }

    /// Returns whether entering `object_id` in `direction` is blocked.
    #[must_use]
    pub fn blocks(&self, object_id: u16, direction: Direction) -> bool {
        self.fixture(object_id)
            .is_some_and(|fixture| fixture.blocks(direction))
    }

    /// Returns the number of decoded static fixtures.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fixtures.len()
    }

    /// Returns whether the catalog has no static fixtures.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fixtures.is_empty()
    }

    /// Returns the source archive path.
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }
}

/// Decodes the one-based variable-length `SObj.tbl` records.
///
/// # Errors
///
/// Returns [`LoadError`] when the table is truncated or structurally invalid.
#[instrument(
    name = "viperzoo::assets::decode_catalog",
    skip(data),
    fields(source = %source.display(), bytes = data.len()),
    err
)]
pub fn decode(data: &[u8], source: PathBuf) -> Result<Catalog, LoadError> {
    let count = read_u32(data, 0).ok_or(LoadError::TableTruncated)?;

    if count == 0 || count > 1_000_000 {
        return Err(LoadError::ObjectCount(count));
    }

    let mut position = HEADER;
    let mut fixtures = Vec::with_capacity(usize::try_from(count).unwrap_or(0));

    for numeric_id in 1..=count {
        let id = u16::try_from(numeric_id).map_err(|_| LoadError::ObjectId(numeric_id))?;
        let prefix = data
            .get(position..position + PREFIX)
            .ok_or(LoadError::ObjectTruncated(id))?;
        let metadata = Metadata::decode(
            prefix[..METADATA]
                .try_into()
                .map_err(|_| LoadError::ObjectTruncated(id))?,
        );
        let collision = Collision::new(prefix[METADATA]);
        let tile_count = usize::from(prefix[METADATA + 1]);

        position += PREFIX;

        let tile_bytes = tile_count
            .checked_mul(2)
            .ok_or(LoadError::ObjectTruncated(id))?;
        let end = position
            .checked_add(tile_bytes)
            .ok_or(LoadError::ObjectTruncated(id))?;

        if end > data.len() {
            return Err(LoadError::ObjectTruncated(id));
        }

        let mut tile_ids = Vec::with_capacity(tile_count);

        for index in 0..tile_count {
            let start = position + index * 2;
            tile_ids.push(u16::from_le_bytes([data[start], data[start + 1]]));
        }

        position = end;
        fixtures.push(Fixture {
            id,
            tile_ids,
            metadata,
            collision,
        });
    }

    if data.len().saturating_sub(position) > 1 {
        return Err(LoadError::Trailing(data.len() - position));
    }

    Ok(Catalog::new(fixtures, source))
}

/// Loads `SObj.tbl` from one explicit `tile.dat` archive.
///
/// # Errors
///
/// Returns [`LoadError`] when the archive or table is invalid.
#[instrument(name = "viperzoo::assets::load_catalog", fields(path = %path.display()), err)]
pub fn load(path: &Path) -> Result<Catalog, LoadError> {
    let data = archive::read_entry(path, "SObj.tbl")?;

    decode(&data, path.to_owned())
}

/// Loads the first current-user or installed `NexusTK` `tile.dat`.
///
/// # Errors
///
/// Returns [`LoadError::NotFound`] when no client archive exists, or another
/// [`LoadError`] when the selected archive is invalid.
#[instrument(name = "viperzoo::assets::load_default_catalog", err)]
pub fn load_default() -> Result<Catalog, LoadError> {
    let mut candidates = Vec::new();

    if let Some(path) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(path).join("KRU/NexusTK/Data/tile.dat"));
    }

    if let Some(path) = env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(path).join("KRU/NexusTK/Data/tile.dat"));
    }

    let Some(path) = candidates.iter().find(|path| path.is_file()) else {
        return Err(LoadError::NotFound(candidates));
    };

    load(path)
}

fn read_u32(data: &[u8], position: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(position..position + 4)?.try_into().ok()?,
    ))
}

/// Static-object catalog failure.
#[derive(Debug, Error)]
pub enum LoadError {
    /// Packed archive could not be decoded.
    #[error(transparent)]
    Archive(#[from] archive::Error),
    /// No candidate client archive exists.
    #[error("could not locate NexusTK Data/tile.dat in {0:?}")]
    NotFound(Vec<PathBuf>),
    /// Table ended before its count and header.
    #[error("SObj.tbl is truncated")]
    TableTruncated,
    /// Declared object count is implausible.
    #[error("SObj.tbl has implausible object count {0}")]
    ObjectCount(u32),
    /// The table's one-based identifier exceeds the protocol field width.
    #[error("SObj.tbl object identifier {0} exceeds u16")]
    ObjectId(u32),
    /// One variable-length record is truncated.
    #[error("SObj.tbl object {0} is truncated")]
    ObjectTruncated(u16),
    /// More than the validated legacy sentinel remains.
    #[error("SObj.tbl has {0} unexplained trailing bytes")]
    Trailing(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a two-record table in the documented prefix-then-tiles order.
    ///
    /// Record 1 carries a present reference; record 2 carries the all-ones
    /// sentinel, which is the shape 97% of the live client's records use.
    fn table() -> Vec<u8> {
        let mut table = 2_u32.to_le_bytes().to_vec();

        table.extend(1_u16.to_le_bytes());

        table.extend(105_u32.to_le_bytes());
        table.push(6);
        table.push(UP);
        table.push(1);
        table.extend(0x1234_u16.to_le_bytes());

        table.extend(Metadata::ABSENT.to_le_bytes());
        table.push(0);
        table.push(0x0f);
        table.push(0);

        table
    }

    #[test]
    fn records_are_one_based_and_directional() {
        let catalog = decode(&table(), "fixture-tile.dat".into()).expect("valid table");

        assert_eq!(
            catalog.fixture(1).expect("first record").tile_ids(),
            &[0x1234]
        );
        assert!(catalog.blocks(1, Direction::Up));
        assert!(!catalog.blocks(1, Direction::Down));
        assert!(
            Direction::VARIANTS
                .iter()
                .all(|direction| catalog.blocks(2, *direction))
        );
    }

    /// Reading the tile list before the fixed prefix consumes the same bytes,
    /// so only per-record attribution reveals the mistake. The first record
    /// owns its own tiles and metadata, not the header's or its successor's.
    #[test]
    fn record_prefix_precedes_its_tile_list() {
        let catalog = decode(&table(), "fixture-tile.dat".into()).expect("valid table");
        let first = catalog.fixture(1).expect("first record");
        let second = catalog.fixture(2).expect("second record");

        assert_eq!(first.metadata().reference(), Some(105));
        assert_eq!(first.metadata().subtype(), 6);
        assert_eq!(first.collision(), Collision::new(UP));
        assert_eq!(first.tile_ids(), &[0x1234]);

        assert_eq!(second.metadata().reference(), None);
        assert_eq!(second.collision(), Collision::new(0x0f));
        assert!(second.tile_ids().is_empty());
    }

    /// The all-ones `u32` is absence, not the value `4294967295`.
    #[test]
    fn sentinel_reference_decodes_as_absent() {
        let present = Metadata::decode([105, 0, 0, 0, 6]);
        let absent = Metadata::decode([0xFF, 0xFF, 0xFF, 0xFF, 0]);

        assert_eq!(present.reference(), Some(105));
        assert_eq!(present.subtype(), 6);
        assert_eq!(absent.reference(), None);
        assert_eq!(absent.subtype(), 0);
    }

    /// Object 2068 is the live table's only record with bits outside the four
    /// cardinal flags. Decoding must keep them rather than normalise to `0x01`.
    #[test]
    fn undecoded_collision_bits_survive_decoding() {
        let outlier = Collision::new(0xF1);

        assert!(outlier.blocks(Direction::Up));
        assert!(!outlier.blocks(Direction::Down));
        assert_eq!(outlier.unknown_bits(), 0xF0);
        assert_eq!(outlier.bits(), 0xF1);
        assert_ne!(outlier, Collision::new(UP));
    }

    #[test]
    fn observed_object_830_blocks_only_northbound_entry() {
        let fixture = Fixture::new(
            830,
            vec![1878, 1874, 1871],
            Metadata::default(),
            Collision::new(UP),
        );
        let catalog = Catalog::new([fixture], "fixture-tile.dat".into());

        assert!(catalog.blocks(830, Direction::Up));
        assert!(!catalog.blocks(830, Direction::Down));
    }
}
