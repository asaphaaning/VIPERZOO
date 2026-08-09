//! Typed boundary for the Python engine's current projection JSON.

use std::{fs, path::Path};

use serde::Deserialize;
use thiserror::Error;

/// Python reference state used for differential checks.
#[derive(Debug, Deserialize)]
pub struct State {
    pub map_epoch: u64,
    pub map: Map,
    pub known_tile_count: usize,
    pub entities: Vec<Entity>,
    pub player: Player,
    pub player_stats: Stats,
    pub player_resources: Resources,
    pub recent_combat_events: Vec<CombatEvent>,
    #[serde(default)]
    pub recent_client_actions: Vec<ClientAction>,
    pub keepalive: Keepalive,
}

impl State {
    /// Reads and parses one complete reference snapshot.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let input = fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_owned(),
            source,
        })?;

        serde_json::from_str(&input).map_err(|source| Error::Json {
            path: path.to_owned(),
            source,
        })
    }

    /// Returns the most recent client-authored combat request retained by Python.
    #[must_use]
    pub fn last_combat_action(&self) -> Option<CombatAction> {
        self.recent_combat_events
            .iter()
            .rev()
            .find_map(CombatEvent::client_action)
    }

    /// Returns the most recent typed client action retained by Python.
    #[must_use]
    pub fn last_client_action(&self) -> Option<ClientAction> {
        self.recent_client_actions.last().cloned()
    }

    /// Returns the evidence class supporting the Python player coordinate.
    #[must_use]
    pub fn position_evidence(&self) -> PositionEvidence {
        match (
            self.player.position_state.as_deref(),
            self.player.position_evidence.as_deref(),
            self.player.status,
        ) {
            (Some("client_reported"), Some("movement"), _) => PositionEvidence::ClientMovement,
            (Some("client_reported"), Some("obstruction"), _) => {
                PositionEvidence::ClientObstruction
            }
            (_, _, Some(_)) => PositionEvidence::Authoritative,
            _ => PositionEvidence::Seed,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Map {
    #[serde(rename = "map_id")]
    pub id: u16,
    pub width: u16,
    pub height: u16,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct Player {
    pub x: u16,
    pub y: u16,
    #[serde(default)]
    position_state: Option<String>,
    #[serde(default)]
    position_evidence: Option<String>,
    #[serde(default)]
    status: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct Stats {
    pub level: u8,
}

#[derive(Debug, Deserialize)]
pub struct Resources {
    pub vita: u32,
    pub max_vita: u32,
    pub mana: u32,
    pub max_mana: u32,
}

#[derive(Debug, Deserialize)]
pub struct Keepalive {
    pub challenges_received: u64,
    pub pongs_sent: u64,
    pub matched_pongs: u64,
}

#[derive(Debug, Deserialize)]
pub struct Entity {}

#[derive(Debug, Deserialize)]
pub struct CombatEvent {
    family: String,
    #[serde(rename = "slot")]
    spell_slot: Option<u8>,
}

impl CombatEvent {
    fn client_action(&self) -> Option<CombatAction> {
        match self.family.as_str() {
            "basic-attack-request" => Some(CombatAction::Attack),
            "spell-cast-request" => Some(CombatAction::Cast {
                slot: self.spell_slot?,
            }),
            _ => None,
        }
    }
}

/// Client-authored combat vocabulary shared by both projections.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CombatAction {
    Attack,
    Cast { slot: u8 },
}

/// Coordinate evidence vocabulary shared by both projections.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionEvidence {
    /// Login, map-entry, or refresh seed.
    Seed,
    /// Client movement prediction.
    ClientMovement,
    /// Client obstruction rollback.
    ClientObstruction,
    /// Server movement reconciliation.
    Authoritative,
}

/// Recent client-action vocabulary shared by both projections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientAction {
    /// One movement request.
    Step {
        /// Requested direction.
        direction: Direction,
        /// Client-reported pre-step tile.
        origin: Position,
        /// Wrapping movement counter.
        last_walk: u8,
    },
    /// One direction-only turn.
    Face {
        /// Requested direction.
        direction: Direction,
    },
    /// One ordinary attack.
    Attack,
    /// One ground-item pickup request.
    Pickup,
    /// One visible-map refresh request.
    Refresh,
    /// One clean client disconnect request.
    Disconnect,
    /// One inventory-slot activation request.
    UseInventory {
        /// One-based inventory slot.
        slot: u8,
    },
    /// One spell invocation.
    Cast {
        /// One-based spellbook slot.
        slot: u8,
    },
    /// One rejected movement edge.
    Obstruction {
        /// Unchanged player tile.
        origin: Position,
        /// Rejected direction.
        direction: Direction,
    },
}

/// Cardinal direction used at the verification boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Negative vertical movement.
    Up,
    /// Positive horizontal movement.
    Right,
    /// Positive vertical movement.
    Down,
    /// Negative horizontal movement.
    Left,
}

/// Coordinate used at the verification boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct Position {
    /// Horizontal tile coordinate.
    x: u16,
    /// Vertical tile coordinate.
    y: u16,
}

impl Position {
    pub(crate) const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

/// Python reference-state boundary failure.
#[derive(Debug, Error)]
pub enum Error {
    #[error("unable to read Python reference state {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("unable to parse Python reference state {path}: {source}")]
    Json {
        path: std::path::PathBuf,
        source: serde_json::Error,
    },
}
