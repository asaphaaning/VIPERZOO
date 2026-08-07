//! Read static object knowledge from versioned `NexusTK` client assets.
//!
//! [`load`] opens the KRU `Data/tile.dat` archive and decodes its `SObj.tbl`
//! member. Each one-based object record contains structurally decoded but
//! semantically unresolved [`Metadata`], a [`Collision`] mask, and visual tile
//! IDs. The result is a [`Catalog`] keyed by the same object ID carried by a
//! live map tile.
//!
//! The catalog does **not** replace or mutate live map knowledge. A streamed
//! region remains authoritative for the coordinate, ground ID, server pass
//! value, and object ID currently present at that coordinate. Navigation joins
//! that live tile to the static catalog only when asking whether it may enter
//! the tile from a particular direction:
//!
//! ```text
//! Live map region                         Static client archive
//! Tile { position, pass_value, object_id }  SObj.tbl[object_id]
//!                  │                                  │
//!                  └───────── navigation ─────────────┘
//!                                   │
//!      blocked when the server pass value, object collision mask, or live
//!      entity occupancy rejects the attempted entry
//! ```
//!
//! This division lets the current network feed describe the world as it is now
//! while the installed client version supplies stable fixture semantics. An
//! unknown asset definition simply contributes no fixture collision knowledge;
//! it never overrides a server-reported blocked tile.

mod archive;
mod object;

pub use archive::{Error, read_entry};
pub use object::{Catalog, Collision, Fixture, LoadError, Metadata, decode, load, load_default};
