//! Own one adapter and its canonical world as a coherent runtime.
//!
//! [`Builder`] is a lazy value: configuring it has no effect until
//! [`Builder::start`] is awaited. A successful start returns the independent
//! application capabilities in one named [`Session`]. Its [`Owner`] is the
//! single teardown authority and always joins both the adapter and engine.
//!
//! ```text
//! Adapter + engine::Config
//!          │
//!     Builder::start
//!          │
//!          ├─ client ──► actions
//!          ├─ events ──► diagnostics
//!          ├─ world  ──► queries
//!          └─ owner  ──► wait | shutdown
//! ```

use std::error::Error as StdError;

use thiserror::Error as ThisError;
use tokio::task::JoinHandle;
use tracing::instrument;
use viperzoo_adapter_api::runtime::{Adapter, Driver, Running};
use viperzoo_engine as engine;

/// Configures a [`Session`] before any runtime work begins.
#[derive(Debug)]
#[must_use = "a session builder has no effect until start is awaited"]
pub struct Builder<A> {
    adapter: A,
    engine: engine::Config,
}

impl<A> Builder<A> {
    /// Creates a session builder around one configured adapter.
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            engine: engine::Config::default(),
        }
    }

    /// Selects the canonical engine configuration.
    pub const fn engine(mut self, engine: engine::Config) -> Self {
        self.engine = engine;
        self
    }
}

impl<A> Builder<A>
where
    A: Adapter,
    A::Error: StdError + Send + Sync + 'static,
{
    /// Starts the engine owner followed by adapter acquisition.
    ///
    /// If adapter startup fails, this method still joins the engine owner
    /// before returning. The caller never receives a partially owned runtime.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when adapter startup fails, the engine owner panics,
    /// or both happen while cleaning up a partial start.
    #[instrument(name = "viperzoo::sdk::session::start", skip(self), err)]
    pub async fn start(self) -> Result<Session<A>, Error<A::Error>> {
        let channel = engine::channel(self.engine);
        let ingress = channel.ingress();
        let world = channel.world();
        let engine = tokio::spawn(channel.owner().run());

        match self.adapter.start(ingress).await {
            Ok(Running {
                client,
                events,
                driver,
            }) => Ok(Session {
                client,
                events,
                world,
                owner: Owner { driver, engine },
            }),
            Err(adapter) => match engine.await {
                Ok(()) => Err(Error::Adapter(adapter)),
                Err(engine) => Err(Error::AdapterAndEngine { adapter, engine }),
            },
        }
    }
}

/// Independent application capabilities of one live adapter session.
#[derive(Debug)]
#[must_use = "a live session must retain its owner until teardown"]
pub struct Session<A>
where
    A: Adapter,
{
    /// Cloneable typed client-action capability.
    pub client: A::Client,
    /// Adapter-specific lifecycle and diagnostic event stream.
    pub events: A::Events,
    /// Cloneable read access to the canonical world.
    pub world: engine::World,
    /// Single lifecycle authority for adapter and engine teardown.
    pub owner: Owner<A::Driver>,
}

impl<A> Session<A>
where
    A: Adapter,
{
    /// Begins configuring a session around one adapter.
    pub fn builder(adapter: A) -> Builder<A> {
        Builder::new(adapter)
    }
}

/// Single lifecycle authority for a [`Session`].
#[derive(Debug)]
#[must_use = "the session owner must be awaited to observe teardown"]
pub struct Owner<D> {
    driver: D,
    engine: JoinHandle<()>,
}

impl<D> Owner<D>
where
    D: Driver,
    D::Error: StdError + Send + Sync + 'static,
{
    /// Returns whether the adapter driver has already stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.driver.is_finished()
    }

    /// Waits for natural adapter termination, then joins the engine owner.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if adapter teardown fails, the engine owner panics,
    /// or both fail.
    #[instrument(name = "viperzoo::sdk::session::wait", skip(self), err)]
    pub async fn wait(self) -> Result<(), Error<D::Error>> {
        let adapter = self.driver.wait().await;
        let engine = self.engine.await;

        finish(adapter, engine)
    }

    /// Requests adapter termination, then joins the engine owner.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if adapter teardown fails, the engine owner panics,
    /// or both fail.
    #[instrument(name = "viperzoo::sdk::session::shutdown", skip(self), err)]
    pub async fn shutdown(self) -> Result<(), Error<D::Error>> {
        let adapter = self.driver.shutdown().await;
        let engine = self.engine.await;

        finish(adapter, engine)
    }
}

fn finish<E>(
    adapter: Result<(), E>,
    engine: Result<(), tokio::task::JoinError>,
) -> Result<(), Error<E>>
where
    E: StdError + 'static,
{
    match (adapter, engine) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(adapter), Ok(())) => Err(Error::Adapter(adapter)),
        (Ok(()), Err(engine)) => Err(Error::Engine(engine)),
        (Err(adapter), Err(engine)) => Err(Error::AdapterAndEngine { adapter, engine }),
    }
}

/// Session startup or teardown failure.
#[derive(Debug, ThisError)]
pub enum Error<E>
where
    E: StdError + 'static,
{
    /// The adapter failed while the engine owner exited normally.
    #[error("adapter runtime failed: {0}")]
    Adapter(E),
    /// The engine owner panicked while the adapter completed normally.
    #[error("engine owner failed: {0}")]
    Engine(tokio::task::JoinError),
    /// Adapter and engine ownership both failed during the same boundary.
    #[error("adapter runtime failed ({adapter}) and engine owner failed ({engine})")]
    AdapterAndEngine {
        /// Adapter startup or teardown failure.
        adapter: E,
        /// Engine owner join failure.
        engine: tokio::task::JoinError,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        pin::Pin,
        task::{Context, Poll},
    };

    use futures_core::Stream;
    use viperzoo_adapter_api::{action, observation};
    use viperzoo_world::revision::Revision;

    use super::*;

    #[derive(Clone, Debug)]
    struct Client;

    impl action::Client for Client {
        type Error = Infallible;

        async fn perform(&self, _action: action::Action) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Events;

    impl Stream for Events {
        type Item = Infallible;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    #[derive(Debug)]
    struct Driver;

    impl viperzoo_adapter_api::runtime::Driver for Driver {
        type Error = Infallible;

        fn is_finished(&self) -> bool {
            false
        }

        async fn wait(self) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn shutdown(self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Fake;

    impl Adapter for Fake {
        type Client = Client;
        type Driver = Driver;
        type Error = Infallible;
        type Event = Infallible;
        type Events = Events;

        async fn start<S>(self, sink: S) -> Result<Running<Client, Events, Driver>, Self::Error>
        where
            S: observation::Sink,
        {
            sink.observe(observation::Observation::SessionStarted)
                .await
                .expect("engine accepts the session boundary");

            Ok(Running::new(Client, Events, Driver))
        }
    }

    #[tokio::test]
    async fn session_owns_adapter_and_engine_as_one_lifecycle() {
        let session = Session::builder(Fake)
            .start()
            .await
            .expect("fake session starts");

        assert_eq!(session.world.latest().revision(), Revision::INITIAL.next());
        session.owner.shutdown().await.expect("session stops");
    }
}
