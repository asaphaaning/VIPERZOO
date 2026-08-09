//! Claimed-field parity between Rust replay and Python current state.

use serde::Serialize;
use viperzoo_replay::capture;
use viperzoo_world::{action, snapshot::Snapshot};

use crate::reference::{self, ClientAction, CombatAction, PositionEvidence};

/// Complete differential verification report.
#[derive(Debug, Serialize)]
pub struct Report {
    passed: bool,
    rust_diagnostics: usize,
    explicit_session_boundaries: u64,
    checks: Checks,
    unsupported_by_rust: [&'static str; 5],
}

impl Report {
    /// Compares every field currently claimed by both projections.
    #[must_use]
    pub fn compare(replay: &capture::Report, reference: &reference::State) -> Self {
        let checks = Checks::compare(replay.snapshot(), reference);
        let passed = replay.diagnostics().is_empty() && checks.passed();

        Self {
            passed,
            rust_diagnostics: replay.diagnostics().len(),
            explicit_session_boundaries: replay.session_start_count(),
            checks,
            unsupported_by_rust: [
                "inventory",
                "spellbook",
                "character_profile_and_equipment",
                "environment_time_and_weather",
                "actor_vitality_and_server_action_effects",
            ],
        }
    }

    /// Returns whether every claimed field matched and no Rust diagnostic occurred.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }
}

#[derive(Debug, Serialize)]
struct Checks {
    map_epoch: Check<u64>,
    map_id: Check<Option<u16>>,
    map_title: Check<Option<String>>,
    map_width: Check<Option<u16>>,
    map_height: Check<Option<u16>>,
    known_tiles: Check<usize>,
    player_x: Check<Option<u16>>,
    player_y: Check<Option<u16>>,
    position_evidence: Check<PositionEvidence>,
    vita: Check<Option<u32>>,
    max_vita: Check<Option<u32>>,
    mana: Check<Option<u32>>,
    max_mana: Check<Option<u32>>,
    level: Check<Option<u8>>,
    visible_entities: Check<usize>,
    heartbeat_challenges: Check<u64>,
    heartbeat_pongs: Check<u64>,
    heartbeat_matches: Check<u64>,
    last_combat_action: Check<Option<CombatAction>>,
    last_client_action: Check<Option<ClientAction>>,
}

impl Checks {
    fn compare(snapshot: &Snapshot, reference: &reference::State) -> Self {
        let context = snapshot.map().context();
        let position = snapshot.player().location().position();
        let resources = snapshot.player().resources();
        let heartbeat = snapshot.heartbeat();

        Self {
            map_epoch: Check::new(snapshot.map().epoch().value(), reference.map_epoch),
            map_id: Check::new(
                context.map(|context| context.id().value()),
                Some(reference.map.id),
            ),
            map_title: Check::new(
                context.and_then(|context| context.title().map(str::to_owned)),
                Some(reference.map.title.clone()),
            ),
            map_width: Check::new(
                context.map(|context| context.dimensions().width()),
                Some(reference.map.width),
            ),
            map_height: Check::new(
                context.map(|context| context.dimensions().height()),
                Some(reference.map.height),
            ),
            known_tiles: Check::new(snapshot.map().tiles().len(), reference.known_tile_count),
            player_x: Check::new(
                position.map(|position| position.x().value()),
                Some(reference.player.x),
            ),
            player_y: Check::new(
                position.map(|position| position.y().value()),
                Some(reference.player.y),
            ),
            position_evidence: Check::new(
                position_evidence(snapshot),
                reference.position_evidence(),
            ),
            vita: Check::new(
                resources.vita().current().value().copied(),
                Some(reference.player_resources.vita),
            ),
            max_vita: Check::new(
                resources.vita().maximum().value().copied(),
                Some(reference.player_resources.max_vita),
            ),
            mana: Check::new(
                resources.mana().current().value().copied(),
                Some(reference.player_resources.mana),
            ),
            max_mana: Check::new(
                resources.mana().maximum().value().copied(),
                Some(reference.player_resources.max_mana),
            ),
            level: Check::new(
                snapshot.player().level().value().copied(),
                Some(reference.player_stats.level),
            ),
            visible_entities: Check::new(snapshot.entities().len(), reference.entities.len()),
            heartbeat_challenges: Check::new(
                heartbeat.challenges_received(),
                reference.keepalive.challenges_received,
            ),
            heartbeat_pongs: Check::new(heartbeat.pongs_observed(), reference.keepalive.pongs_sent),
            heartbeat_matches: Check::new(
                heartbeat.matched_pongs(),
                reference.keepalive.matched_pongs,
            ),
            last_combat_action: Check::new(
                last_combat_action(snapshot),
                reference.last_combat_action(),
            ),
            last_client_action: Check::new(
                last_client_action(snapshot),
                reference.last_client_action(),
            ),
        }
    }

    fn passed(&self) -> bool {
        self.map_epoch.passed()
            && self.map_id.passed()
            && self.map_title.passed()
            && self.map_width.passed()
            && self.map_height.passed()
            && self.known_tiles.passed()
            && self.player_x.passed()
            && self.player_y.passed()
            && self.position_evidence.passed()
            && self.vita.passed()
            && self.max_vita.passed()
            && self.mana.passed()
            && self.max_mana.passed()
            && self.level.passed()
            && self.visible_entities.passed()
            && self.heartbeat_challenges.passed()
            && self.heartbeat_pongs.passed()
            && self.heartbeat_matches.passed()
            && self.last_combat_action.passed()
            && self.last_client_action.passed()
    }
}

fn position_evidence(snapshot: &Snapshot) -> PositionEvidence {
    match snapshot.player().location() {
        viperzoo_world::player::Location::Unknown
        | viperzoo_world::player::Location::Seeded { .. } => PositionEvidence::Seed,
        viperzoo_world::player::Location::ClientReported { evidence, .. } => match evidence {
            viperzoo_world::player::ClientEvidence::Movement { .. } => {
                PositionEvidence::ClientMovement
            }
            viperzoo_world::player::ClientEvidence::Obstruction { .. } => {
                PositionEvidence::ClientObstruction
            }
        },
        viperzoo_world::player::Location::Authoritative { .. } => PositionEvidence::Authoritative,
    }
}

fn last_client_action(snapshot: &Snapshot) -> Option<ClientAction> {
    snapshot
        .recent_actions()
        .last()
        .and_then(|event| match event.action() {
            action::Action::Step {
                direction,
                origin,
                last_walk,
            } => Some(ClientAction::Step {
                direction: direction_from_protocol(*direction),
                origin: reference::Position::new(origin.x().value(), origin.y().value()),
                last_walk: *last_walk,
            }),
            action::Action::Face { direction } => Some(ClientAction::Face {
                direction: direction_from_protocol(*direction),
            }),
            action::Action::Attack => Some(ClientAction::Attack),
            action::Action::Pickup => Some(ClientAction::Pickup),
            action::Action::Refresh => Some(ClientAction::Refresh),
            action::Action::Disconnect => Some(ClientAction::Disconnect),
            action::Action::UseInventory { slot } => {
                Some(ClientAction::UseInventory { slot: *slot })
            }
            action::Action::Cast { slot } => Some(ClientAction::Cast { slot: *slot }),
            action::Action::Obstruction { origin, direction } => Some(ClientAction::Obstruction {
                origin: reference::Position::new(origin.x().value(), origin.y().value()),
                direction: direction_from_protocol(*direction),
            }),
            // The Python reference reducer predates the observed bank and
            // travel vocabulary. Treat its absence as unsupported rather than
            // comparing a fabricated surrogate action.
            action::Action::Speak { .. }
            | action::Action::Interact { .. }
            | action::Action::Dialog { .. }
            | action::Action::TravelSelection { .. } => None,
        })
}

const fn direction_from_protocol(
    direction: viperzoo_protocol::direction::Direction,
) -> reference::Direction {
    match direction {
        viperzoo_protocol::direction::Direction::Up => reference::Direction::Up,
        viperzoo_protocol::direction::Direction::Right => reference::Direction::Right,
        viperzoo_protocol::direction::Direction::Down => reference::Direction::Down,
        viperzoo_protocol::direction::Direction::Left => reference::Direction::Left,
    }
}

fn last_combat_action(snapshot: &Snapshot) -> Option<CombatAction> {
    snapshot
        .recent_combat_actions()
        .iter()
        .rev()
        .find_map(|event| match event.action() {
            action::Action::Attack => Some(CombatAction::Attack),
            action::Action::Cast { slot } => Some(CombatAction::Cast { slot: *slot }),
            action::Action::Step { .. }
            | action::Action::Face { .. }
            | action::Action::Pickup
            | action::Action::Refresh
            | action::Action::Disconnect
            | action::Action::UseInventory { .. }
            | action::Action::Obstruction { .. }
            | action::Action::Speak { .. }
            | action::Action::Interact { .. }
            | action::Action::Dialog { .. }
            | action::Action::TravelSelection { .. } => None,
        })
}

#[derive(Debug, Serialize)]
struct Check<T> {
    status: Status,
    rust: T,
    python: T,
}

impl<T: Eq> Check<T> {
    fn new(rust: T, python: T) -> Self {
        let status = if rust == python {
            Status::Match
        } else {
            Status::Mismatch
        };

        Self {
            status,
            rust,
            python,
        }
    }

    fn passed(&self) -> bool {
        self.status == Status::Match
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Match,
    Mismatch,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("replay")
            .join("tests")
            .join("data")
            .join(name)
    }

    fn foundation_reference(known_tile_count: usize) -> reference::State {
        serde_json::from_value(serde_json::json!({
            "map_epoch": 1,
            "map": {
                "map_id": 4711,
                "width": 17,
                "height": 17,
                "title": "Welcome"
            },
            "known_tile_count": known_tile_count,
            "entities": [{}],
            "player": { "x": 3, "y": 1 },
            "player_stats": { "level": 3 },
            "player_resources": {
                "vita": 151,
                "max_vita": 151,
                "mana": 98,
                "max_mana": 98
            },
            "recent_combat_events": [],
            "recent_client_actions": [],
            "keepalive": {
                "challenges_received": 0,
                "pongs_sent": 0,
                "matched_pongs": 0
            }
        }))
        .expect("reference fixture has the production schema")
    }

    #[tokio::test]
    async fn passes_only_when_every_claimed_field_matches() {
        let path = fixture("foundation.jsonl");
        let input = fs::read(&path).expect("fixture is readable");
        let replay = capture::replay(input.as_slice(), &path)
            .await
            .expect("fixture replays");

        assert!(Report::compare(&replay, &foundation_reference(2)).passed());
        assert!(!Report::compare(&replay, &foundation_reference(3)).passed());
    }
}
