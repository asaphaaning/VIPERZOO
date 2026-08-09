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

use std::{num::NonZeroUsize, sync::Arc};

use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{debug, instrument};
use viperzoo_adapter_api::observation::Observation;
use viperzoo_world::{snapshot::Snapshot, world::Change};

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
/// let _snapshot = world.snapshot();
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

/// Cloneable read access to the canonical world of a running [`Owner`].
#[derive(Clone, Debug)]
pub struct World {
    snapshots: watch::Receiver<Arc<Snapshot>>,
}

impl World {
    /// Returns the latest internally consistent snapshot without waiting.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshots.borrow())
    }

    /// Subscribes to future canonical snapshot revisions.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshots.clone()
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

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{decode, direction::Flow, primitive::Position};

    use super::*;

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
            world.snapshot().player().location().position(),
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

        assert_eq!(snapshots.borrow().revision().value(), 1);

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

        assert_eq!(world.snapshot().revision().value(), 0);
    }
}
