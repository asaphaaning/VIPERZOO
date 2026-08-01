//! Confirm a face-and-attack transaction from canonical observations.
//!
//! Submitting an action to an adapter proves only that the native client
//! accepted the request. This module waits for the projected outbound attack
//! body before reporting success. Facing is handled separately because the
//! client can represent a direction key against a blocking fixture as an
//! obstruction instead of an explicit facing packet.

use std::{fmt, time::Duration};

use thiserror::Error;
use tokio::{sync::watch, time};
use tracing::instrument;
use viperzoo_adapter_api::action::{self, Client};
use viperzoo_engine::Handle;
use viperzoo_protocol::direction::Direction;
use viperzoo_world::{action as observed, revision::Revision, snapshot::Snapshot};

/// Timing policy for one face-and-attack transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    observation_timeout: Duration,
}

impl Config {
    /// Creates a combat action policy with an explicit observation timeout.
    #[must_use]
    pub const fn new(observation_timeout: Duration) -> Self {
        Self {
            observation_timeout,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(Duration::from_secs(2))
    }
}

/// Canonical revisions proving one facing request and attack were observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Report {
    facing: Facing,
    attack: Revision,
}

/// How the requested attack direction was established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Facing {
    /// The canonical projection already held the requested direction.
    Retained,
    /// A new facing packet established the requested direction.
    Observed(Revision),
    /// The native directional request was accepted by the adapter, but the
    /// client represented an adjacent blocking fixture as an obstruction
    /// rather than an explicit plaintext facing packet. The following attack
    /// body and target effect remain mandatory proof of the transaction.
    Submitted,
}

impl Report {
    /// Returns how the requested direction was established.
    #[must_use]
    pub const fn facing(self) -> Facing {
        self.facing
    }

    /// Returns the revision that observed the attack request.
    #[must_use]
    pub const fn attack(self) -> Revision {
        self.attack
    }
}

/// One phase whose outbound plaintext observation is required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// Direction-only facing request.
    Facing,
    /// Ordinary attack request.
    Attack,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Facing => formatter.write_str("facing"),
            Self::Attack => formatter.write_str("attack"),
        }
    }
}

/// Faces `direction`, attacks, and confirms the actual outbound attack body.
///
/// Adapter submission alone is insufficient: the returned [`Report`] proves
/// the canonical engine observed serverbound `0x13`. `NexusTK` may represent a
/// directional key aimed at a blocking fixture as obstruction rather than a
/// serverbound `0x11`; that state is recorded as [`Facing::Submitted`] and
/// must be followed by higher-level target-effect confirmation.
///
/// # Errors
///
/// Returns [`enum@Error`] when the adapter fails, the engine stops, or the
/// mandatory plaintext attack body is absent before the configured timeout.
#[instrument(
    name = "viperzoo::actions::combat::face_and_attack",
    skip(client, engine),
    fields(?direction),
    err,
    ret(level = "debug")
)]
pub async fn face_and_attack<C>(
    client: &C,
    engine: &Handle,
    direction: Direction,
    config: Config,
) -> Result<Report, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    let mut snapshots = engine.subscribe();
    let before = snapshots.borrow().revision();
    let facing = if snapshots.borrow().player().facing().value() == Some(&direction) {
        Facing::Retained
    } else {
        client
            .perform(action::Action::Face(direction))
            .await
            .map_err(Error::Client)?;
        match wait_for_action(
            &mut snapshots,
            before,
            Stage::Facing,
            config.observation_timeout,
            |action| {
                matches!(action, observed::Action::Face { direction: value } if *value == direction)
            },
        )
        .await
        {
            Ok(revision) => {
                if let Some(attack) = find_action(&snapshots.borrow(), before, &|action| {
                    matches!(action, observed::Action::Attack)
                }) {
                    return Ok(Report {
                        facing: Facing::Submitted,
                        attack,
                    });
                }
                Facing::Observed(revision)
            }
            Err(Error::Unobserved(Stage::Facing)) => {
                if let Some(attack) = find_action(&snapshots.borrow(), before, &|action| {
                    matches!(action, observed::Action::Attack)
                }) {
                    return Ok(Report {
                        facing: Facing::Submitted,
                        attack,
                    });
                }
                Facing::Submitted
            }
            Err(error) => return Err(error),
        }
    };
    // A directional key pressed into an adjacent tree can itself produce the
    // client's native attack body while it fails to produce `0x11`. Fence the
    // explicit attack after the facing transaction so that incidental action
    // cannot confirm the following intent.
    let before_attack = snapshots.borrow().revision();
    client
        .perform(action::Action::Attack(direction))
        .await
        .map_err(Error::Client)?;
    let attack = wait_for_action(
        &mut snapshots,
        before_attack,
        Stage::Attack,
        config.observation_timeout,
        |action| matches!(action, observed::Action::Attack),
    )
    .await?;

    Ok(Report { facing, attack })
}

async fn wait_for_action<E>(
    snapshots: &mut watch::Receiver<std::sync::Arc<Snapshot>>,
    after: Revision,
    stage: Stage,
    timeout: Duration,
    matches: impl Fn(&observed::Action) -> bool,
) -> Result<Revision, Error<E>>
where
    E: fmt::Debug + fmt::Display,
{
    if let Some(revision) = find_action(&snapshots.borrow(), after, &matches) {
        return Ok(revision);
    }

    time::timeout(timeout, async {
        loop {
            snapshots
                .changed()
                .await
                .map_err(|_| Error::EngineStopped)?;

            if let Some(revision) = find_action(&snapshots.borrow(), after, &matches) {
                return Ok(revision);
            }
        }
    })
    .await
    .map_err(|_| Error::Unobserved(stage))?
}

fn find_action(
    snapshot: &Snapshot,
    after: Revision,
    matches: &impl Fn(&observed::Action) -> bool,
) -> Option<Revision> {
    snapshot
        .recent_actions()
        .iter()
        .rev()
        .find(|event| event.revision() > after && matches(event.action()))
        .map(observed::Event::revision)
}

/// Confirmed combat-action failure.
#[derive(Debug, Error)]
pub enum Error<E>
where
    E: fmt::Debug + fmt::Display,
{
    /// The client action adapter rejected submission.
    #[error("client action failed: {0}")]
    Client(E),
    /// The canonical engine stopped before observing an action.
    #[error("canonical engine stopped during combat action confirmation")]
    EngineStopped,
    /// The adapter returned without the expected outbound plaintext body.
    #[error("submitted {0} action was not observed at the plaintext protocol boundary")]
    Unobserved(Stage),
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicU8, Ordering},
        },
    };

    use viperzoo_engine::Config as EngineConfig;
    use viperzoo_protocol::{decode, direction::Flow};

    use super::*;

    #[derive(Clone)]
    struct ObservingClient {
        engine: Handle,
    }

    #[derive(Clone)]
    struct AttackOnlyClient {
        engine: Handle,
    }

    #[derive(Clone)]
    struct DirectionalAttackClient {
        engine: Handle,
        submissions: Arc<AtomicU8>,
    }

    impl Client for ObservingClient {
        type Error = Infallible;

        async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
            let body = match action {
                action::Action::Face(direction) => vec![0x11, direction.to_wire(), 0x00],
                action::Action::Attack(_) => vec![0x13, 0x00, 0x00],
                _ => return Ok(()),
            };
            let packet = decode(Flow::Serverbound, &body).expect("fixture action decodes");
            self.engine
                .observe(packet.into())
                .await
                .expect("test engine remains available");
            Ok(())
        }
    }

    impl Client for AttackOnlyClient {
        type Error = Infallible;

        async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
            if !matches!(action, action::Action::Attack(_)) {
                return Ok(());
            }

            let packet =
                decode(Flow::Serverbound, &[0x13, 0x00, 0x00]).expect("fixture action decodes");
            self.engine
                .observe(packet.into())
                .await
                .expect("test engine remains available");
            Ok(())
        }
    }

    impl Client for DirectionalAttackClient {
        type Error = Infallible;

        async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
            if !matches!(action, action::Action::Face(_) | action::Action::Attack(_)) {
                return Ok(());
            }

            let packet =
                decode(Flow::Serverbound, &[0x13, 0x00, 0x00]).expect("fixture action decodes");
            self.engine
                .observe(packet.into())
                .await
                .expect("test engine remains available");
            self.submissions.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn report_requires_observed_facing_then_attack() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());
        let client = ObservingClient {
            engine: engine.clone(),
        };

        let report = face_and_attack(&client, &engine, Direction::Right, Config::default())
            .await
            .expect("both submitted bodies are observed");

        let Facing::Observed(facing) = report.facing() else {
            panic!("unknown initial facing requires an observed facing packet");
        };
        assert!(facing < report.attack());
        assert_eq!(
            engine.snapshot().player().facing().value(),
            Some(&Direction::Right)
        );

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }

    #[tokio::test]
    async fn retained_facing_avoids_a_redundant_packet() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());
        let client = ObservingClient {
            engine: engine.clone(),
        };

        face_and_attack(&client, &engine, Direction::Right, Config::default())
            .await
            .expect("initial transaction establishes facing");
        let before = engine.snapshot().revision();
        let report = face_and_attack(&client, &engine, Direction::Right, Config::default())
            .await
            .expect("retained facing permits attack");

        assert_eq!(report.facing(), Facing::Retained);
        assert_eq!(report.attack().value(), before.value() + 1);

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }

    #[tokio::test]
    async fn obstruction_style_native_facing_can_be_proven_by_following_attack() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());
        let client = AttackOnlyClient {
            engine: engine.clone(),
        };

        let report = face_and_attack(&client, &engine, Direction::Up, Config::new(Duration::ZERO))
            .await
            .expect("an observed native attack completes the transaction");

        assert_eq!(report.facing(), Facing::Submitted);

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }

    #[tokio::test]
    async fn directional_wake_attack_fulfills_the_single_swing_intent() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());
        let client = DirectionalAttackClient {
            engine: engine.clone(),
            submissions: Arc::new(AtomicU8::new(0)),
        };

        let report = face_and_attack(&client, &engine, Direction::Up, Config::new(Duration::ZERO))
            .await
            .expect("the native directional attack fulfills the transaction");

        assert_eq!(report.facing(), Facing::Submitted);
        assert_eq!(report.attack().value(), 1);
        assert_eq!(client.submissions.load(Ordering::Relaxed), 1);

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }
}
