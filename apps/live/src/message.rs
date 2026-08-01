//! Machine-readable live projection output.

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use viperzoo_capture::record::Diagnostic;
use viperzoo_protocol::primitive::Position;
use viperzoo_world::{action, snapshot::Snapshot};

/// One published live-engine event.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message<'a> {
    /// Historical catch-up completed with compact operational state.
    #[serde(rename = "ready")]
    ReadySummary {
        /// Compact projection at the catch-up boundary.
        summary: Summary<'a>,
    },
    /// Historical catch-up completed with a complete world projection.
    #[serde(rename = "ready")]
    ReadySnapshot {
        /// Complete projection at the catch-up boundary.
        snapshot: &'a Snapshot,
    },
    /// A live packet changed compact operational state.
    #[serde(rename = "projected")]
    ProjectedSummary {
        /// Compact projection after the change.
        summary: Summary<'a>,
    },
    /// A live packet changed the complete world projection.
    #[serde(rename = "projected")]
    ProjectedSnapshot {
        /// Complete projection after the change.
        snapshot: &'a Snapshot,
    },
    /// One acquisition row was quarantined.
    Diagnostic {
        /// Boundary failure that did not mutate canonical state.
        diagnostic: &'a Diagnostic,
    },
}

/// Compact terminal-friendly projection facts.
#[derive(Debug, Serialize)]
pub struct Summary<'a> {
    revision: u64,
    processed_packets: u64,
    unknown_packets: u64,
    map: Map<'a>,
    position: Option<Position>,
    vita: Pool,
    mana: Pool,
    level: Option<u8>,
    visible_entities: usize,
    heartbeat: Heartbeat,
    recent_actions: usize,
    last_action: Option<&'a action::Event>,
}

impl<'a> Summary<'a> {
    /// Extracts compact facts from one internally consistent [`Snapshot`].
    #[must_use]
    pub fn from_snapshot(snapshot: &'a Snapshot) -> Self {
        let resources = snapshot.player().resources();
        let heartbeat = snapshot.heartbeat();

        Self {
            revision: snapshot.revision().value(),
            processed_packets: snapshot.processed_packet_count(),
            unknown_packets: snapshot.unknown_packet_count(),
            map: Map::from_snapshot(snapshot.map()),
            position: snapshot.player().location().position(),
            vita: Pool::from_knowledge(resources.vita()),
            mana: Pool::from_knowledge(resources.mana()),
            level: snapshot.player().level().value().copied(),
            visible_entities: snapshot.entities().len(),
            heartbeat: Heartbeat {
                challenges: heartbeat.challenges_received(),
                pongs: heartbeat.pongs_observed(),
                matched_pongs: heartbeat.matched_pongs(),
            },
            recent_actions: snapshot.recent_actions().len(),
            last_action: snapshot.recent_actions().last(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Map<'a> {
    Unidentified {
        epoch: u64,
        known_tiles: usize,
    },
    Identified {
        epoch: u64,
        id: u16,
        title: &'a str,
        width: u16,
        height: u16,
        known_tiles: usize,
    },
}

impl<'a> Map<'a> {
    fn from_snapshot(snapshot: &'a viperzoo_world::map::Snapshot) -> Self {
        let epoch = snapshot.epoch().value();
        let known_tiles = snapshot.tiles().len();

        match snapshot.context() {
            Some(context) => Self::Identified {
                epoch,
                id: context.id().value(),
                title: context.title(),
                width: context.dimensions().width(),
                height: context.dimensions().height(),
                known_tiles,
            },
            None => Self::Unidentified { epoch, known_tiles },
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Pool {
    current: Option<u32>,
    maximum: Option<u32>,
}

impl Pool {
    fn from_knowledge(pool: &viperzoo_world::player::Pool) -> Self {
        Self {
            current: pool.current().value().copied(),
            maximum: pool.maximum().value().copied(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Heartbeat {
    challenges: u64,
    pongs: u64,
    matched_pongs: u64,
}

/// Writes one newline-delimited JSON [`Message`].
pub async fn write(
    writer: &mut (impl AsyncWrite + Unpin),
    message: Message<'_>,
) -> Result<(), Error> {
    let mut encoded = serde_json::to_vec(&message)?;
    encoded.push(b'\n');
    writer.write_all(&encoded).await?;
    writer.flush().await?;

    Ok(())
}

/// Live message publication failure.
#[derive(Debug, Error)]
pub enum Error {
    /// A typed message could not be represented as JSON.
    #[error("unable to serialize live engine message: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The machine-readable output stream rejected a complete message.
    #[error("unable to write live engine message: {0}")]
    Write(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn summary_messages_are_small_complete_json_lines() {
        let snapshot = viperzoo_world::world::World::new().snapshot();
        let mut output = Vec::new();

        write(
            &mut output,
            Message::ReadySummary {
                summary: Summary::from_snapshot(&snapshot),
            },
        )
        .await
        .expect("memory output accepts a complete message");

        assert!(output.ends_with(b"\n"));
        assert!(output.len() < 512);
        let value: serde_json::Value =
            serde_json::from_slice(&output).expect("message is valid JSON");
        assert_eq!(value["type"], "ready");
        assert!(value["summary"].get("map").is_some());
    }

    #[tokio::test]
    async fn snapshot_mode_preserves_the_original_event_vocabulary() {
        let snapshot = viperzoo_world::world::World::new().snapshot();
        let mut output = Vec::new();

        write(
            &mut output,
            Message::ReadySnapshot {
                snapshot: &snapshot,
            },
        )
        .await
        .expect("memory output accepts a complete snapshot");

        let value: serde_json::Value =
            serde_json::from_slice(&output).expect("message is valid JSON");
        assert_eq!(value["type"], "ready");
        assert!(value["snapshot"].get("map").is_some());
    }
}
