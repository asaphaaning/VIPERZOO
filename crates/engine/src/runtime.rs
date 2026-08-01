//! Serialize live observations through one bounded asynchronous owner.
//!
//! [`Handle`] is cloneable so adapters and controllers can submit observations
//! concurrently. [`Task`] receives them through a bounded channel, applies
//! them in order through [`crate::Reducer`], replies with a [`Receipt`], and
//! publishes only complete `Arc<Snapshot>` revisions through a watch channel.
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

/// Cloneable command and snapshot handle for a running [`Task`].
#[derive(Clone, Debug)]
pub struct Handle {
    commands: mpsc::Sender<Command>,
    snapshots: watch::Receiver<Arc<Snapshot>>,
}

impl Handle {
    /// Orders one observation and waits until it is reflected in snapshots.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stopped`] if the owner [`Task`] has stopped.
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
    /// Use [`Handle::observe`] there instead.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stopped`] if the owner [`Task`] has stopped.
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

    /// Requests an orderly engine stop after all prior commands.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stopped`] if the owner [`Task`] has already stopped.
    #[instrument(
        name = "viperzoo::engine::shutdown",
        skip(self),
        err,
        ret(level = "debug")
    )]
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.commands
            .send(Command::Shutdown)
            .await
            .map_err(|_| Error::Stopped)
    }
}

/// Lazy single-owner engine task.
///
/// Nothing is reduced until [`Task::run`] is awaited or spawned.
#[derive(Debug)]
#[must_use = "the engine task must be run for its handle to make progress"]
pub struct Task {
    commands: mpsc::Receiver<Command>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    reducer: Reducer,
}

impl Task {
    /// Owns and reduces observations until shutdown or the last [`Handle`] is dropped.
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
                Command::Shutdown => break,
            }
        }
    }
}

/// Creates a connected [`Handle`] and lazy owner [`Task`].
#[instrument(
    name = "viperzoo::engine::channel",
    fields(capacity = config.capacity().get()),
    ret(level = "trace")
)]
pub fn channel(config: Config) -> (Handle, Task) {
    let reducer = Reducer::new();
    let (commands, receiver) = mpsc::channel(config.capacity().get());
    let (snapshots, subscription) = watch::channel(Arc::new(reducer.snapshot()));

    (
        Handle {
            commands,
            snapshots: subscription,
        },
        Task {
            commands: receiver,
            snapshots,
            reducer,
        },
    )
}

const fn observation_name(observation: &Observation) -> &'static str {
    match observation {
        Observation::SessionStarted => "session-started",
        Observation::TransportClosed => "transport-closed",
        Observation::Packet(_) => "packet",
        Observation::PlayerResources(_) => "player-resources",
        Observation::PlayerInventory(_) => "player-inventory",
    }
}

#[derive(Debug)]
enum Command {
    Observe {
        observation: Box<Observation>,
        reply: oneshot::Sender<Receipt>,
    },
    Shutdown,
}

/// Engine command failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The owner task stopped before accepting or acknowledging the command.
    #[error("engine task has stopped")]
    Stopped,
}

#[cfg(test)]
mod tests {
    use viperzoo_protocol::{decode, direction::Flow, primitive::Position};

    use super::*;

    #[tokio::test]
    async fn observations_are_acknowledged_after_snapshot_publication() {
        let (handle, task) = channel(Config::default());
        let owner = tokio::spawn(task.run());
        let bytes = hex::decode("04000300010003000100410000").expect("fixture hex is valid");
        let packet = decode(Flow::Clientbound, &bytes).expect("fixture packet is valid");

        let receipt = handle
            .observe(packet.into())
            .await
            .expect("engine is running");

        assert!(receipt.change().is_projected());
        assert_eq!(
            handle.snapshot().player().location().position(),
            Some(Position::new(3, 1))
        );

        handle.shutdown().await.expect("engine is running");
        owner.await.expect("engine task does not panic");
    }

    #[tokio::test]
    async fn subscribers_receive_each_canonical_revision() {
        let (handle, task) = channel(Config::default());
        let owner = tokio::spawn(task.run());
        let mut snapshots = handle.subscribe();

        handle
            .observe(Observation::SessionStarted)
            .await
            .expect("engine is running");
        snapshots.changed().await.expect("publisher is alive");

        assert_eq!(snapshots.borrow().revision().value(), 1);

        handle.shutdown().await.expect("engine is running");
        owner.await.expect("engine task does not panic");
    }
}
