//! Walk toward a destination by treating every movement as new evidence.
//!
//! A route is only a statement about one snapshot. This controller requests one
//! client-native step, waits for movement or obstruction evidence, learns a
//! repeated silent refusal as a directed edge, then plans again. It therefore
//! adapts to streamed tiles and moving entities instead of executing a stale
//! route as a blind command queue.
//!
//! The target is bound to the map epoch established during bootstrap. If a map
//! change occurs, the transaction stops rather than applying the same numeric
//! coordinate to a different map.

use std::{fmt, time::Duration};

use thiserror::Error;
use tokio::{sync::watch, time};
use tracing::{debug, instrument};
use viperzoo_adapter_api::action::{self, Client};
use viperzoo_assets::Catalog;
use viperzoo_engine::Handle;
use viperzoo_navigation::{Edge, Knowledge, Plan, plan, plan_with_assets};
use viperzoo_protocol::{direction::Direction, primitive::Position};
use viperzoo_world::snapshot::Snapshot;

const EDGE_OBSERVATIONS: u8 = 3;
const REVALIDATION_EPOCHS: u8 = 3;
const REVALIDATION_DELAY: Duration = Duration::from_millis(1_500);

/// Destination-controller timing and safety limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    readiness_timeout: Duration,
    step_timeout: Duration,
    settle_delay: Duration,
    maximum_attempts: u32,
}

impl Config {
    /// Creates a configuration with explicit bounds.
    #[must_use]
    pub const fn new(
        readiness_timeout: Duration,
        step_timeout: Duration,
        settle_delay: Duration,
        maximum_attempts: u32,
    ) -> Self {
        Self {
            readiness_timeout,
            step_timeout,
            settle_delay,
            maximum_attempts,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(3),
            Duration::from_millis(1_500),
            Duration::from_millis(120),
            512,
        )
    }
}

/// Successful destination-walking outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Report {
    target: Position,
    attempts: u32,
    learned_edges: u32,
}

impl Report {
    /// Returns the reached destination.
    #[must_use]
    pub const fn target(self) -> Position {
        self.target
    }

    /// Returns submitted client-step attempts.
    #[must_use]
    pub const fn attempts(self) -> u32 {
        self.attempts
    }

    /// Returns how many silent refusal edges were learned during the run.
    #[must_use]
    pub const fn learned_edges(self) -> u32 {
        self.learned_edges
    }
}

/// Walks to `target`, replanning after every projected state transition.
///
/// If a warm attachment lacks map or position state, the controller first asks
/// the normal client for a map refresh. Moving entities and newly merged map
/// tiles are considered on every iteration. A step that produces no movement
/// or obstruction evidence before the bounded timeout becomes a learned
/// directional collision for the current map epoch. The destination itself is
/// scoped to the epoch observed after bootstrap; a later epoch change aborts
/// the transaction instead of reusing the same numeric coordinate on another
/// map.
///
/// # Errors
///
/// Returns [`enum@Error`] for adapter failures, unavailable state, planning
/// failures, engine shutdown, or exhausted safety limits.
#[instrument(
    name = "viperzoo::actions::walk_to",
    skip(client, engine),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err,
    ret(level = "debug")
)]
pub async fn to<C>(
    client: &C,
    engine: &Handle,
    target: Position,
    config: Config,
) -> Result<Report, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    run(client, engine, target, config, None).await
}

/// Walks to `target` with client-asset directional fixture collision.
///
/// # Errors
///
/// Returns the same [`enum@Error`] vocabulary as [`to`].
#[instrument(
    name = "viperzoo::actions::walk_to_with_assets",
    skip(client, engine, assets),
    fields(target_x = target.x().value(), target_y = target.y().value()),
    err,
    ret(level = "debug")
)]
pub async fn to_with_assets<C>(
    client: &C,
    engine: &Handle,
    assets: &Catalog,
    target: Position,
    config: Config,
) -> Result<Report, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    run(client, engine, target, config, Some(assets)).await
}

#[instrument(
    name = "viperzoo::actions::walk::run",
    skip(client, engine, assets),
    fields(target_x = target.x().value(), target_y = target.y().value(), assets = assets.is_some()),
    err,
    ret(level = "debug")
)]
async fn run<C>(
    client: &C,
    engine: &Handle,
    target: Position,
    config: Config,
    assets: Option<&Catalog>,
) -> Result<Report, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    let mut snapshots = engine.subscribe();
    let mut attempts = bootstrap(client, &mut snapshots, config).await?;
    let epoch = snapshots.borrow().map().epoch();
    let mut knowledge = Knowledge::new();
    let mut learned_edges = 0_u32;
    let mut revalidation_epochs = 0_u8;

    'walk: loop {
        let snapshot = snapshots.borrow().clone();

        if snapshot.map().epoch() != epoch {
            return Err(Error::MapChanged {
                from: epoch.value(),
                to: snapshot.map().epoch().value(),
            });
        }

        let planned = assets.map_or_else(
            || plan(&snapshot, &knowledge, target),
            |assets| plan_with_assets(&snapshot, &knowledge, assets, target),
        );
        let route = match planned {
            Err(viperzoo_navigation::Error::NoRoute { .. })
                if !knowledge.is_empty() && revalidation_epochs < REVALIDATION_EPOCHS =>
            {
                revalidation_epochs += 1;
                debug!(
                    epoch = revalidation_epochs,
                    "runtime refusal edges eliminated every route; waiting before revalidation"
                );
                time::sleep(REVALIDATION_DELAY).await;
                knowledge.clear();
                continue;
            }
            Err(error) => return Err(Error::Plan(error)),
            Ok(Plan::Route(route) | Plan::Frontier(route)) => route,
            Ok(Plan::Arrived) => {
                return Ok(Report {
                    target,
                    attempts,
                    learned_edges,
                });
            }
        };

        let origin = snapshot
            .player()
            .location()
            .position()
            .ok_or(Error::StateUnavailable)?;
        let direction = route.first().ok_or(Error::RouteExhausted)?;
        let edge = Edge::new(origin, direction);
        let destination = edge.destination().ok_or(Error::StateUnavailable)?;
        for observation in 1..=EDGE_OBSERVATIONS {
            if attempts == config.maximum_attempts {
                return Err(Error::Attempts(config.maximum_attempts));
            }

            attempts += 1;
            debug!(
                attempt = attempts,
                observation,
                x = origin.x().value(),
                y = origin.y().value(),
                ?direction,
                route_length = route.len(),
                "submitting client-native step"
            );

            client
                .perform(action::Action::Step(direction))
                .await
                .map_err(Error::Client)?;

            match wait_for_step(&mut snapshots, origin, destination, config.step_timeout).await {
                Ok(()) => {
                    time::sleep(config.settle_delay).await;
                    continue 'walk;
                }
                Err(WaitError::Stopped) => return Err(Error::EngineStopped),
                Err(WaitError::Timeout)
                    if snapshots.borrow().entities_at(destination).next().is_some() =>
                {
                    debug!(
                        x = destination.x().value(),
                        y = destination.y().value(),
                        "destination became occupied; leaving edge unclassified"
                    );
                    time::sleep(config.settle_delay).await;
                    continue 'walk;
                }
                Err(WaitError::Timeout) if observation < EDGE_OBSERVATIONS => {
                    time::sleep(config.settle_delay).await;
                }
                Err(WaitError::Timeout) => {}
            }
        }

        if knowledge.block(edge) {
            learned_edges += 1;
        }
    }
}

#[instrument(
    name = "viperzoo::actions::walk::bootstrap",
    skip(client, snapshots),
    err,
    ret(level = "debug")
)]
async fn bootstrap<C>(
    client: &C,
    snapshots: &mut watch::Receiver<std::sync::Arc<Snapshot>>,
    config: Config,
) -> Result<u32, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    // A map-context packet after a portal transition normally already carries
    // the player's projected position.  Do not add a force-response/refresh
    // round trip in that latency-sensitive window: it is both unnecessary and
    // creates avoidable client activity during the transition.
    if localized(&snapshots.borrow()) {
        return Ok(0);
    }

    client
        .perform(action::Action::MapData(action::MapData::ForceResponse))
        .await
        .map_err(Error::Client)?;

    if !localized(&snapshots.borrow()) {
        client
            .perform(action::Action::RefreshMap)
            .await
            .map_err(Error::Client)?;

        match wait_until(snapshots, config.readiness_timeout, localized).await {
            Err(WaitError::Stopped) => return Err(Error::EngineStopped),
            Ok(()) | Err(WaitError::Timeout) => {}
        }
    }

    if localized(&snapshots.borrow()) {
        Ok(0)
    } else {
        localize(client, snapshots, config.step_timeout).await
    }
}

fn localized(snapshot: &Snapshot) -> bool {
    snapshot.player().location().position().is_some()
}

#[instrument(
    name = "viperzoo::actions::walk::localize",
    skip(client, snapshots),
    err,
    ret(level = "debug")
)]
async fn localize<C>(
    client: &C,
    snapshots: &mut watch::Receiver<std::sync::Arc<Snapshot>>,
    timeout: Duration,
) -> Result<u32, Error<C::Error>>
where
    C: Client,
    C::Error: fmt::Debug + fmt::Display,
{
    let mut attempts = 0_u32;

    for direction in Direction::VARIANTS.iter().copied() {
        attempts += 1;
        debug!(
            attempt = attempts,
            ?direction,
            "probing live client localization"
        );
        client
            .perform(action::Action::Step(direction))
            .await
            .map_err(Error::Client)?;

        match wait_until(snapshots, timeout, localized).await {
            Ok(()) => return Ok(attempts),
            Err(WaitError::Stopped) => return Err(Error::EngineStopped),
            Err(WaitError::Timeout) => {}
        }
    }

    Err(Error::StateUnavailable)
}

async fn wait_for_step(
    snapshots: &mut watch::Receiver<std::sync::Arc<Snapshot>>,
    origin: Position,
    destination: Position,
    timeout: Duration,
) -> Result<(), WaitError> {
    wait_until(snapshots, timeout, |snapshot| {
        let position = snapshot.player().location().position();

        position == Some(destination)
            || position.is_some_and(|position| position != origin)
            || matches!(
                snapshot.player().location(),
                viperzoo_world::player::Location::ClientReported {
                    position,
                    evidence: viperzoo_world::player::ClientEvidence::Obstruction { .. },
                    ..
                } if *position == origin
            )
    })
    .await
}

async fn wait_until(
    snapshots: &mut watch::Receiver<std::sync::Arc<Snapshot>>,
    timeout: Duration,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Result<(), WaitError> {
    if predicate(&snapshots.borrow()) {
        return Ok(());
    }

    time::timeout(timeout, async {
        loop {
            snapshots.changed().await.map_err(|_| WaitError::Stopped)?;

            if predicate(&snapshots.borrow()) {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| WaitError::Timeout)?
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitError {
    Stopped,
    Timeout,
}

/// Destination-controller failure.
#[derive(Debug, Error)]
pub enum Error<E>
where
    E: fmt::Debug + fmt::Display,
{
    /// The client-native action adapter failed.
    #[error("client action failed: {0}")]
    Client(E),
    /// Player position remained unavailable after refresh and bounded probes.
    #[error("player position is unavailable after refresh and four live localization probes")]
    StateUnavailable,
    /// The canonical engine stopped during the run.
    #[error("canonical engine stopped during destination walking")]
    EngineStopped,
    /// The map identity epoch changed before the map-scoped target was reached.
    #[error("map epoch changed from {from} to {to}; destination is no longer valid")]
    MapChanged {
        /// Epoch against which the target was planned.
        from: u64,
        /// Newly observed epoch that invalidated the target.
        to: u64,
    },
    /// Current projected state admits no route.
    #[error(transparent)]
    Plan(#[from] viperzoo_navigation::Error),
    /// A planner returned a route that had no executable step.
    #[error("planner returned an exhausted route")]
    RouteExhausted,
    /// The controller reached its configured safety limit.
    #[error("destination walking exceeded {0} attempts")]
    Attempts(u32),
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use viperzoo_engine::Config as EngineConfig;
    use viperzoo_protocol::{decode, direction::Flow};

    use super::*;

    #[derive(Clone)]
    struct TransitioningClient {
        engine: Handle,
    }

    #[derive(Clone, Default)]
    struct CountingClient {
        map_data_requests: Arc<AtomicUsize>,
        refresh_requests: Arc<AtomicUsize>,
    }

    impl Client for CountingClient {
        type Error = Infallible;

        async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
            match action {
                action::Action::MapData(_) => {
                    self.map_data_requests.fetch_add(1, Ordering::Relaxed);
                }
                action::Action::RefreshMap => {
                    self.refresh_requests.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }

            Ok(())
        }
    }

    impl Client for TransitioningClient {
        type Error = Infallible;

        async fn perform(&self, action: action::Action) -> Result<(), Self::Error> {
            if !matches!(action, action::Action::Step(_)) {
                return Ok(());
            }

            for body in [
                "1512680011001105000757656c636f6d6500e80002020200",
                "04000500050003000100410000",
            ] {
                let bytes = hex::decode(body).expect("fixture hex is valid");
                let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet decodes");
                self.engine
                    .observe(packet.into())
                    .await
                    .expect("test engine remains available");
            }

            Ok(())
        }
    }

    #[tokio::test]
    async fn map_transition_invalidates_numeric_destination() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());

        for body in [
            "1512670011001105000757656c636f6d6500e80002020200",
            "04000300010003000100410000",
        ] {
            let bytes = hex::decode(body).expect("fixture hex is valid");
            let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet decodes");
            engine
                .observe(packet.into())
                .await
                .expect("test engine remains available");
        }

        let client = TransitioningClient {
            engine: engine.clone(),
        };
        let result = to(&client, &engine, Position::new(3, 0), Config::default()).await;

        assert!(matches!(result, Err(Error::MapChanged { from: 1, to: 2 })));

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }

    #[tokio::test]
    async fn localized_bootstrap_does_not_refresh_the_new_map() {
        let (engine, task) = viperzoo_engine::channel(EngineConfig::default());
        let owner = tokio::spawn(task.run());

        for body in [
            "1512670011001105000757656c636f6d6500e80002020200",
            "04000300010003000100410000",
        ] {
            let bytes = hex::decode(body).expect("fixture hex is valid");
            let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet decodes");
            engine
                .observe(packet.into())
                .await
                .expect("test engine remains available");
        }

        let client = CountingClient::default();
        let mut snapshots = engine.subscribe();
        let attempts = bootstrap(&client, &mut snapshots, Config::default())
            .await
            .expect("localized projection needs no bootstrap action");

        assert_eq!(attempts, 0);
        assert_eq!(client.map_data_requests.load(Ordering::Relaxed), 0);
        assert_eq!(client.refresh_requests.load(Ordering::Relaxed), 0);

        engine.shutdown().await.expect("engine shuts down");
        owner.await.expect("engine owner joins");
    }
}
