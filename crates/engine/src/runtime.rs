//! Serialize live observations through one bounded asynchronous owner.
//!
//! [`Channel`] separates the three capabilities of a live engine. Cloneable
//! [`Ingress`] values submit observations, cloneable [`World`] values read
//! immutable snapshots, and one [`Owner`] reduces every accepted observation.
//! The types make observation authority, knowledge access, and runtime
//! ownership distinct instead of granting all three to every caller.
//!
//! Backpressure is intentional: an acquisition boundary must slow down rather
//! than let an unbounded queue hide lost responsiveness or memory growth.

use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, instrument};
use viperzoo_adapter_api::observation::{self, Observation};
use viperzoo_world::{
    query::{Query, State},
    snapshot::Snapshot,
    world::Change,
};

use crate::Reducer;

/// Bounded engine runtime configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    capacity: NonZeroUsize,
}

impl Config {
    /// Creates a configuration with the given bounded command capacity.
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self { capacity }
    }

    /// Returns the maximum number of queued commands.
    #[must_use]
    pub const fn capacity(self) -> NonZeroUsize {
        self.capacity
    }
}

impl Default for Config {
    fn default() -> Self {
        let capacity = NonZeroUsize::new(256).unwrap_or(NonZeroUsize::MIN);

        Self::new(capacity)
    }
}

/// Receipt proving that one observation was reduced in sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receipt {
    change: Change,
}

impl Receipt {
    /// Returns the canonical effect of the observation.
    #[must_use]
    pub const fn change(self) -> Change {
        self.change
    }
}

/// The connected capabilities of one live engine.
///
/// Clone the capabilities needed by each participant before consuming the
/// channel with [`Channel::owner`]:
///
/// ```no_run
/// # async fn run() {
/// let channel = viperzoo_engine::channel(viperzoo_engine::Config::default());
/// let ingress = channel.ingress();
/// let world = channel.world();
/// let owner = tokio::spawn(channel.owner().run());
///
/// drop(ingress);
/// owner.await.expect("engine owner does not panic");
///
/// let _snapshot = world.latest();
/// # }
/// ```
#[derive(Debug)]
#[must_use = "the engine channel must be divided into the capabilities its runtime needs"]
pub struct Channel {
    ingress: Ingress,
    world: World,
    owner: Owner,
}

impl Channel {
    /// Returns a cloneable observation capability.
    #[must_use]
    pub fn ingress(&self) -> Ingress {
        self.ingress.clone()
    }

    /// Returns a cloneable canonical-world capability.
    #[must_use]
    pub fn world(&self) -> World {
        self.world.clone()
    }

    /// Consumes the channel and returns its single runtime owner.
    pub fn owner(self) -> Owner {
        self.owner
    }
}

/// Cloneable authority to submit observations to a running [`Owner`].
#[derive(Clone, Debug)]
pub struct Ingress {
    commands: mpsc::Sender<Command>,
}

impl Ingress {
    /// Orders one observation and waits until it is reflected in snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stopped`] if the [`Owner`] has stopped.
    #[instrument(
        name = "viperzoo::engine::observe",
        skip(self, observation),
        fields(observation = observation_name(&observation)),
        err,
        ret(level = "debug")
    )]
    pub async fn observe(&self, observation: Observation) -> Result<Receipt, Error> {
        let (reply, receipt) = oneshot::channel();

        self.commands
            .send(Command::Observe {
                observation: Box::new(observation),
                reply,
            })
            .await
            .map_err(|_| Error::Stopped)?;

        receipt.await.map_err(|_| Error::Stopped)
    }

    /// Orders one observation from a synchronous adapter thread.
    ///
    /// # Panics
    ///
    /// Panics when called from within an asynchronous Tokio execution context.
    /// Use [`Ingress::observe`] there instead.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stopped`] if the [`Owner`] has stopped.
    #[instrument(
        name = "viperzoo::engine::observe_blocking",
        skip(self, observation),
        fields(observation = observation_name(&observation)),
        err,
        ret(level = "debug")
    )]
    pub fn observe_blocking(&self, observation: Observation) -> Result<Receipt, Error> {
        let (reply, receipt) = oneshot::channel();

        self.commands
            .blocking_send(Command::Observe {
                observation: Box::new(observation),
                reply,
            })
            .map_err(|_| Error::Stopped)?;

        receipt.blocking_recv().map_err(|_| Error::Stopped)
    }
}

impl observation::Sink for Ingress {
    async fn observe(&self, observation: Observation) -> Result<(), observation::Error> {
        Ingress::observe(self, observation)
            .await
            .map(|_| ())
            .map_err(|Error::Stopped| observation::Error::Closed)
    }

    fn observe_blocking(&self, observation: Observation) -> Result<(), observation::Error> {
        Ingress::observe_blocking(self, observation)
            .map(|_| ())
            .map_err(|Error::Stopped| observation::Error::Closed)
    }
}

/// Cloneable read access to the canonical world of a running [`Owner`].
#[derive(Clone, Debug)]
pub struct World {
    snapshots: watch::Receiver<Arc<Snapshot>>,
}

impl World {
    /// Returns the latest internally consistent snapshot without waiting.
    #[must_use]
    pub fn latest(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshots.borrow())
    }

    /// Atomically captures the latest snapshot and future revisions.
    ///
    /// [`Subscription::latest`] and the first [`Subscription::changed`] call
    /// meet without a gap or duplicate revision.
    #[must_use]
    pub fn subscribe(&self) -> Subscription {
        let mut snapshots = self.snapshots.clone();
        let latest = Arc::clone(&snapshots.borrow_and_update());

        Subscription { latest, snapshots }
    }

    /// Prepares to resolve a pure world [`Query`] over current and future
    /// revisions.
    pub fn wait<Q>(&self, query: Q) -> Wait<'static, Q>
    where
        Q: Query,
    {
        Wait {
            source: Source::Owned(self.subscribe()),
            query,
        }
    }
}

/// Gap-free view of one current snapshot followed by future revisions.
#[derive(Debug)]
pub struct Subscription {
    latest: Arc<Snapshot>,
    snapshots: watch::Receiver<Arc<Snapshot>>,
}

impl Subscription {
    /// Returns the latest snapshot observed by this subscription.
    #[must_use]
    pub fn latest(&self) -> Arc<Snapshot> {
        Arc::clone(&self.latest)
    }

    /// Waits for and returns the next canonical snapshot revision.
    ///
    /// # Errors
    ///
    /// Returns [`SubscriptionError::Closed`] when the engine owner stops.
    pub async fn changed(&mut self) -> Result<Arc<Snapshot>, SubscriptionError> {
        self.snapshots
            .changed()
            .await
            .map_err(|_| SubscriptionError::Closed)?;
        self.latest = Arc::clone(&self.snapshots.borrow_and_update());

        Ok(self.latest())
    }

    /// Prepares to resolve a [`Query`] without losing this subscription's
    /// current revision fence.
    pub fn wait<Q>(&mut self, query: Q) -> Wait<'_, Q>
    where
        Q: Query,
    {
        Wait {
            source: Source::Borrowed(self),
            query,
        }
    }
}

/// A lazy query wait over current and future world revisions.
#[derive(Debug)]
#[must_use = "a world wait has no effect until run or within is awaited"]
pub struct Wait<'a, Q> {
    source: Source<'a>,
    query: Q,
}

impl<Q> Wait<'_, Q>
where
    Q: Query,
{
    /// Waits without a deadline until the query resolves.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Closed`] when the engine owner stops first.
    pub async fn run(mut self) -> Result<Q::Output, WaitError> {
        loop {
            if let State::Ready(value) = self.query.evaluate(&self.source.latest()) {
                return Ok(value);
            }

            self.source.changed().await?;
        }
    }

    /// Waits up to `duration` for the query to resolve.
    ///
    /// # Errors
    ///
    /// Returns [`WaitError::Elapsed`] when the deadline passes or
    /// [`WaitError::Closed`] when the engine owner stops first.
    pub async fn within(self, duration: Duration) -> Result<Q::Output, WaitError> {
        tokio::time::timeout(duration, self.run())
            .await
            .map_err(|_| WaitError::Elapsed { duration })?
    }
}

#[derive(Debug)]
enum Source<'a> {
    Owned(Subscription),
    Borrowed(&'a mut Subscription),
}

impl Source<'_> {
    fn latest(&self) -> Arc<Snapshot> {
        match self {
            Self::Owned(subscription) => subscription.latest(),
            Self::Borrowed(subscription) => subscription.latest(),
        }
    }

    async fn changed(&mut self) -> Result<Arc<Snapshot>, SubscriptionError> {
        match self {
            Self::Owned(subscription) => subscription.changed().await,
            Self::Borrowed(subscription) => subscription.changed().await,
        }
    }
}

/// Lazy single-owner engine runtime.
///
/// Nothing is reduced until [`Owner::run`] is awaited or spawned. The owner
/// drains every accepted observation and finishes after the last [`Ingress`]
/// is dropped.
#[derive(Debug)]
#[must_use = "the engine owner must be run for its ingress to make progress"]
pub struct Owner {
    commands: mpsc::Receiver<Command>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    reducer: Reducer,
}

impl Owner {
    /// Reduces observations until every [`Ingress`] has been dropped.
    #[instrument(name = "viperzoo::engine::run", skip(self))]
    pub async fn run(mut self) {
        while let Some(command) = self.commands.recv().await {
            match command {
                Command::Observe { observation, reply } => {
                    let change = self.reducer.observe(*observation);
                    self.snapshots
                        .send_replace(Arc::new(self.reducer.snapshot()));
                    let _ = reply.send(Receipt { change });

                    debug!(
                        revision = change.revision().value(),
                        projected = change.is_projected(),
                        "ordered observation reduced"
                    );
                }
            }
        }
    }
}

/// Creates the connected capabilities of one live engine.
#[instrument(
    name = "viperzoo::engine::channel",
    fields(capacity = config.capacity().get()),
    ret(level = "trace")
)]
pub fn channel(config: Config) -> Channel {
    let reducer = Reducer::new();
    let (commands, receiver) = mpsc::channel(config.capacity().get());
    let (snapshots, subscription) = watch::channel(Arc::new(reducer.snapshot()));

    Channel {
        ingress: Ingress { commands },
        world: World {
            snapshots: subscription,
        },
        owner: Owner {
            commands: receiver,
            snapshots,
            reducer,
        },
    }
}

const fn observation_name(observation: &Observation) -> &'static str {
    match observation {
        Observation::SessionStarted => "session-started",
        Observation::TransportClosed => "transport-closed",
        Observation::Packet(_) => "packet",
        Observation::PlayerResources(_) => "player-resources",
        Observation::PlayerInventory(_) => "player-inventory",
        Observation::ClientMap { .. } => "client-map",
    }
}

#[derive(Debug)]
enum Command {
    Observe {
        observation: Box<Observation>,
        reply: oneshot::Sender<Receipt>,
    },
}

/// Engine command failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The owner stopped before accepting or acknowledging the observation.
    #[error("engine owner has stopped")]
    Stopped,
}

/// Failure while advancing an atomic [`Subscription`].
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SubscriptionError {
    /// The engine owner stopped before another revision was published.
    #[error("canonical world subscription is closed")]
    Closed,
}

impl From<SubscriptionError> for WaitError {
    fn from(error: SubscriptionError) -> Self {
        match error {
            SubscriptionError::Closed => Self::Closed,
        }
    }
}

/// Failure while waiting for a world [`Query`].
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum WaitError {
    /// The engine owner stopped before the query resolved.
    #[error("canonical world closed before the query resolved")]
    Closed,
    /// The query remained pending for the complete deadline.
    #[error("world query did not resolve within {duration:?}")]
    Elapsed {
        /// Configured wait duration.
        duration: Duration,
    },
}

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{
        codec::{Body, Error as CodecError},
        direction::Flow,
        packet,
        primitive::Position,
    };
    use viperzoo_world::{query, revision::Revision};

    use super::*;

    fn decode(flow: Flow, body: &[u8]) -> Result<packet::Packet, CodecError> {
        packet::Packet::try_from(Body::new(flow, body))
    }

    #[tokio::test]
    async fn observations_are_acknowledged_after_snapshot_publication() {
        let channel = channel(Config::default());
        let ingress = channel.ingress();
        let world = channel.world();
        let owner = tokio::spawn(channel.owner().run());
        let bytes = hex::decode("04000300010003000100410000").expect("fixture hex is valid");
        let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet is valid");

        let receipt = ingress
            .observe(packet.into())
            .await
            .expect("engine is running");

        assert!(receipt.change().is_projected());
        assert_eq!(
            world.latest().player().location().position(),
            Some(Position::new(3, 1))
        );

        drop(ingress);
        owner.await.expect("engine owner does not panic");
    }

    #[tokio::test]
    async fn subscribers_receive_the_latest_published_revision() {
        let channel = channel(Config::default());
        let ingress = channel.ingress();
        let world = channel.world();
        let owner = tokio::spawn(channel.owner().run());
        let mut snapshots = world.subscribe();

        ingress
            .observe(Observation::SessionStarted)
            .await
            .expect("engine is running");
        snapshots.changed().await.expect("publisher is alive");

        assert_eq!(snapshots.latest().revision().value(), 1);

        drop(ingress);
        owner.await.expect("engine owner does not panic");
    }

    #[tokio::test]
    async fn subscription_snapshot_and_future_revisions_meet_without_a_gap() {
        let channel = channel(Config::default());
        let ingress = channel.ingress();
        let world = channel.world();
        let owner = tokio::spawn(channel.owner().run());

        ingress
            .observe(Observation::SessionStarted)
            .await
            .expect("engine is running");

        let mut subscription = world.subscribe();
        assert_eq!(subscription.latest().revision().value(), 1);

        ingress
            .observe(Observation::TransportClosed)
            .await
            .expect("engine is running");
        let changed = subscription.changed().await.expect("publisher is alive");
        assert_eq!(changed.revision().value(), 2);

        drop(ingress);
        owner.await.expect("engine owner does not panic");
    }

    #[tokio::test]
    async fn queries_resolve_immediately_from_the_atomic_current_snapshot() {
        let channel = channel(Config::default());
        let ingress = channel.ingress();
        let world = channel.world();
        let owner = tokio::spawn(channel.owner().run());

        ingress
            .observe(Observation::SessionStarted)
            .await
            .expect("engine is running");

        let revision = world
            .wait(query::after(Revision::INITIAL))
            .within(Duration::from_millis(10))
            .await
            .expect("current snapshot already resolves the query");
        assert_eq!(revision.value(), 1);

        drop(ingress);
        owner.await.expect("engine owner does not panic");
    }

    #[tokio::test]
    async fn world_readers_do_not_keep_the_owner_alive() {
        let channel = channel(Config::default());
        let ingress = channel.ingress();
        let world = channel.world();
        let owner = tokio::spawn(channel.owner().run());

        drop(ingress);
        owner.await.expect("engine owner does not panic");

        assert_eq!(world.latest().revision().value(), 0);
    }
}
