//! Decode `SObj.tbl` records into directional fixture knowledge.
//!
//! A live map tile gives a one-based object ID, not the record's contents.
//! `SObj.tbl` supplies those contents: visual tile IDs, five opaque metadata
//! bytes, and one collision byte. [`Catalog`] keeps this static data separate
//! from the live world, then uses the object ID as the join key when navigation
//! evaluates a current tile.
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

/// One static fixture decoded from a one-based `SObj.tbl` record.
///
/// A live map tile refers to a fixture only by its [`Fixture::id`]. This value
/// provides the client-version-specific details for that identifier: its visual
/// tile IDs, uninterpreted metadata, and directional collision mask. It is not
/// a live world object and does not carry a position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fixture {
    id: u16,
    tile_ids: Vec<u16>,
    metadata: [u8; 5],
    collision: u8,
}

impl Fixture {
    /// Creates one decoded or synthetic static fixture.
    #[must_use]
    pub fn new(id: u16, tile_ids: Vec<u16>, metadata: [u8; 5], collision: u8) -> Self {
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

    /// Returns the unresolved five-byte metadata block.
    #[must_use]
    pub const fn metadata(&self) -> [u8; 5] {
        self.metadata
    }

    /// Returns the raw directional collision mask.
    #[must_use]
    pub const fn collision_flags(&self) -> u8 {
        self.collision
    }

    /// Returns whether entering this fixture in `direction` is blocked.
    #[must_use]
    pub const fn blocks(&self, direction: Direction) -> bool {
        let bit = match direction {
            Direction::Up => UP,
            Direction::Right => RIGHT,
            Direction::Down => DOWN,
            Direction::Left => LEFT,
        };

        self.collision & bit != 0
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

    let mut position = 5_usize;
    let mut fixtures = Vec::with_capacity(usize::try_from(count).unwrap_or(0));

    for numeric_id in 1..=count {
        let id = u16::try_from(numeric_id).map_err(|_| LoadError::ObjectId(numeric_id))?;
        let tile_count = usize::from(*data.get(position).ok_or(LoadError::ObjectTruncated(id))?);
        position += 1;
        let tile_bytes = tile_count
            .checked_mul(2)
            .ok_or(LoadError::ObjectTruncated(id))?;
        let end = position
            .checked_add(tile_bytes + 6)
            .ok_or(LoadError::ObjectTruncated(id))?;

        if end > data.len() {
            return Err(LoadError::ObjectTruncated(id));
        }

        let mut tile_ids = Vec::with_capacity(tile_count);

        for index in 0..tile_count {
            let start = position + index * 2;
            tile_ids.push(u16::from_le_bytes([data[start], data[start + 1]]));
        }

        position += tile_bytes;
        let metadata = data[position..position + 5]
            .try_into()
            .map_err(|_| LoadError::ObjectTruncated(id))?;
        position += 5;
        let collision = data[position];
        position += 1;
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

    #[test]
    fn records_are_one_based_and_directional() {
        let mut table = 2_u32.to_le_bytes().to_vec();
        table.push(0);
        table.push(1);
        table.extend(0x1234_u16.to_le_bytes());
        table.extend(*b"abcde");
        table.push(UP);
        table.push(0);
        table.extend(*b"fghij");
        table.push(0x0f);
        table.push(0);

        let catalog = decode(&table, "fixture-tile.dat".into()).expect("valid table");

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

    #[test]
    fn observed_object_830_blocks_only_northbound_entry() {
        let fixture = Fixture::new(830, vec![1878, 1874, 1871], [0; 5], UP);
        let catalog = Catalog::new([fixture], "fixture-tile.dat".into());

        assert!(catalog.blocks(830, Direction::Up));
        assert!(!catalog.blocks(830, Direction::Down));
    }
}
