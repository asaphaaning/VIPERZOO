//! Own one adapter and its canonical world as a coherent runtime.
//!
//! [`Builder`] is a lazy value: configuring it has no effect until
//! [`Builder::start`] is awaited. A successful start returns the independent
//! application capabilities in one named [`Session`]. Its [`Owner`] is the
//! single teardown authority and always joins both the adapter and engine.
//!
//! ```text
//! session() + Adapter + engine::Config
//!          │
//!     Builder::start
//!          │
//!          ├─ client ──► actions
//!          ├─ events ──► diagnostics
//!          ├─ world  ──► queries
//!          └─ owner  ──► wait | shutdown
//! ```
//!
//! Applications may retain the adapter event stream from [`Session`] or make
//! delivery part of ownership with [`Builder::on_event`]:
//!
//! ```text
//! session()
//!     .adapter(adapter)
//!     .on_event(|event| match event { /* exhaustive adapter vocabulary */ })
//!     .start()
//! ```

use std::{error::Error as StdError, time::Duration};

use thiserror::Error as ThisError;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tracing::{Instrument, instrument};
use viperzoo_adapter_api::runtime::{Adapter, Driver, Running as AdapterRunning};
use viperzoo_engine as engine;

const LIFECYCLE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Begins configuring an opinionated adapter and engine [`Session`].
///
/// The returned builder has no adapter and therefore cannot be started. Use
/// [`Builder::adapter`] to complete the runtime shape.
#[must_use = "a session builder has no effect until an adapter is selected and start is awaited"]
pub fn session() -> Builder {
    Builder::new()
}

/// Configures a [`Session`] before any runtime work begins.
#[derive(Debug)]
#[must_use = "a session builder has no effect until start is awaited"]
pub struct Builder<A = ()> {
    adapter: A,
    engine: engine::Config,
}

impl Builder {
    fn new() -> Self {
        Self {
            adapter: (),
            engine: engine::Config::default(),
        }
    }

    /// Selects the adapter that will acquire observations and perform actions.
    pub fn adapter<A>(self, adapter: A) -> Builder<A> {
        Builder {
            adapter,
            engine: self.engine,
        }
    }
}

impl<A> Builder<A> {
    /// Selects the canonical engine configuration.
    pub const fn engine(mut self, engine: engine::Config) -> Self {
        self.engine = engine;
        self
    }
}

impl<A> Builder<A>
where
    A: Adapter,
{
    /// Delegates every typed adapter event to `handler` for the session lifetime.
    ///
    /// The handler runs on an SDK-owned task. The returned
    /// [`HandledSession`] has no event stream because its [`Owner`] now owns
    /// event delivery and joins the handler during teardown. Handlers should
    /// remain brief and non-blocking; applications needing asynchronous event
    /// policy should retain the stream returned by [`Builder::start`].
    pub fn on_event<F>(self, handler: F) -> HandledBuilder<A, F>
    where
        F: FnMut(A::Event) + Send + 'static,
    {
        HandledBuilder {
            builder: self,
            handler,
        }
    }

    /// Explicitly drains and discards adapter events for the session lifetime.
    pub fn discard_events(self) -> HandledBuilder<A, impl FnMut(A::Event) + Send + 'static> {
        self.on_event(drop::<A::Event>)
    }
}

/// A complete session builder with SDK-owned typed event delivery.
#[derive(Debug)]
#[must_use = "a handled session builder has no effect until start is awaited"]
pub struct HandledBuilder<A, F> {
    builder: Builder<A>,
    handler: F,
}

impl<A, F> HandledBuilder<A, F> {
    /// Selects the canonical engine configuration.
    pub fn engine(mut self, engine: engine::Config) -> Self {
        self.builder = self.builder.engine(engine);
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
    /// Returns [`Error`] when adapter startup fails, the engine task fails,
    /// or both happen while cleaning up a partial start.
    #[instrument(name = "viperzoo::sdk::session::start", skip(self), err)]
    pub async fn start(self) -> Result<Session<A>, Error<A::Error>> {
        let Started {
            client,
            events,
            driver,
            world,
            engine,
        } = start(self).await?;

        Ok(Session {
            client,
            events,
            world,
            owner: Owner::new(driver, engine, None),
        })
    }
}

impl<A, F> HandledBuilder<A, F>
where
    A: Adapter,
    A::Error: StdError + Send + Sync + 'static,
    F: FnMut(A::Event) + Send + 'static,
{
    /// Starts the engine and adapter with SDK-owned event delivery.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when engine scheduling or adapter startup fails, or
    /// when the engine task fails while cleaning up a partial start.
    #[instrument(name = "viperzoo::sdk::session::start_handled", skip(self), err)]
    pub async fn start(self) -> Result<HandledSession<A>, Error<A::Error>> {
        let Started {
            client,
            events,
            driver,
            world,
            engine,
        } = start(self.builder).await?;
        let events = EventTask::spawn(events, self.handler);

        Ok(HandledSession {
            client,
            world,
            owner: Owner::new(driver, engine, Some(events)),
        })
    }
}

struct Started<A>
where
    A: Adapter,
{
    client: A::Client,
    events: A::Events,
    driver: A::Driver,
    world: engine::World,
    engine: engine::Task,
}

async fn start<A>(builder: Builder<A>) -> Result<Started<A>, Error<A::Error>>
where
    A: Adapter,
    A::Error: StdError + Send + Sync + 'static,
{
    let engine = engine::channel(builder.engine)
        .spawn()
        .map_err(Error::EngineStart)?;
    let ingress = engine.ingress();
    let world = engine.world();
    let engine = engine.owner();

    match builder.adapter.start(ingress).await {
        Ok(AdapterRunning {
            client,
            events,
            driver,
        }) => Ok(Started {
            client,
            events,
            driver,
            world,
            engine,
        }),
        Err(adapter) => match engine.wait().await {
            Ok(()) => Err(Error::Adapter(adapter)),
            Err(engine) => Err(Error::AdapterAndEngine { adapter, engine }),
        },
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

/// Consumer capabilities of a session whose events are handled by the SDK.
#[derive(Debug)]
#[must_use = "a live handled session must retain its owner until teardown"]
pub struct HandledSession<A>
where
    A: Adapter,
{
    /// Cloneable typed client-action capability.
    pub client: A::Client,
    /// Cloneable read access to the canonical world.
    pub world: engine::World,
    /// Single lifecycle authority for adapter, event, and engine teardown.
    pub owner: Owner<A::Driver>,
}

/// Single lifecycle authority for a [`Session`].
#[derive(Debug)]
#[must_use = "the session owner must be awaited to observe teardown"]
pub struct Owner<D> {
    driver: D,
    engine: engine::Task,
    events: Option<EventTask>,
}

impl<D> Owner<D> {
    const fn new(driver: D, engine: engine::Task, events: Option<EventTask>) -> Self {
        Self {
            driver,
            engine,
            events,
        }
    }
}

#[derive(Debug)]
struct EventTask {
    handle: JoinHandle<()>,
}

impl EventTask {
    fn spawn<E, F>(mut events: E, mut handler: F) -> Self
    where
        E: tokio_stream::Stream + Send + Unpin + 'static,
        E::Item: Send + 'static,
        F: FnMut(E::Item) + Send + 'static,
    {
        let task = async move {
            while let Some(event) = events.next().await {
                handler(event);
            }
        }
        .instrument(tracing::info_span!("viperzoo::sdk::session::event_handler"));

        Self {
            handle: tokio::spawn(task),
        }
    }

    #[instrument(name = "viperzoo::sdk::session::events::wait", skip(self), err)]
    async fn wait(self) -> Result<(), EventError> {
        self.handle.await.map_err(EventError::Join)
    }
}

impl<D> Owner<D>
where
    D: Driver,
    D::Error: StdError + Send + Sync + 'static,
{
    /// Returns whether the adapter driver has already stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.driver.is_finished() || self.engine.is_finished()
    }

    /// Waits until the adapter or canonical engine has finished.
    ///
    /// This borrows the owner so it can be selected alongside application
    /// work. Call [`Owner::wait`] or [`Owner::shutdown`] afterward to consume
    /// ownership and surface the final lifecycle result.
    #[instrument(name = "viperzoo::sdk::session::finished", skip(self))]
    pub async fn finished(&self) {
        while !self.is_finished() {
            tokio::time::sleep(LIFECYCLE_POLL_INTERVAL).await;
        }
    }

    /// Waits for natural adapter termination, then joins the engine owner.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if adapter teardown, typed event delivery, or the
    /// engine task fails. Combined failures retain every failed authority.
    #[instrument(name = "viperzoo::sdk::session::wait", skip(self), err)]
    pub async fn wait(self) -> Result<(), Error<D::Error>> {
        let Self {
            driver,
            engine,
            events,
        } = self;
        let adapter = driver.wait().await;
        let events = wait_events(events).await;
        let engine = engine.wait().await;

        finish(adapter, events, engine)
    }

    /// Requests adapter termination, then joins the engine owner.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if adapter teardown, typed event delivery, or the
    /// engine task fails. Combined failures retain every failed authority.
    #[instrument(name = "viperzoo::sdk::session::shutdown", skip(self), err)]
    pub async fn shutdown(self) -> Result<(), Error<D::Error>> {
        let Self {
            driver,
            engine,
            events,
        } = self;
        let adapter = driver.shutdown().await;
        let events = wait_events(events).await;
        let engine = engine.wait().await;

        finish(adapter, events, engine)
    }
}

async fn wait_events(events: Option<EventTask>) -> Result<(), EventError> {
    match events {
        Some(events) => events.wait().await,
        None => Ok(()),
    }
}

fn finish<E>(
    adapter: Result<(), E>,
    events: Result<(), EventError>,
    engine: Result<(), engine::TaskError>,
) -> Result<(), Error<E>>
where
    E: StdError + 'static,
{
    match (adapter, events, engine) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(adapter), Ok(()), Ok(())) => Err(Error::Adapter(adapter)),
        (Ok(()), Err(events), Ok(())) => Err(Error::Events(events)),
        (Ok(()), Ok(()), Err(engine)) => Err(Error::Engine(engine)),
        (Err(adapter), Err(events), Ok(())) => Err(Error::AdapterAndEvents { adapter, events }),
        (Err(adapter), Ok(()), Err(engine)) => Err(Error::AdapterAndEngine { adapter, engine }),
        (Ok(()), Err(events), Err(engine)) => Err(Error::EventsAndEngine { events, engine }),
        (Err(adapter), Err(events), Err(engine)) => Err(Error::AdapterEventsAndEngine {
            adapter,
            events,
            engine,
        }),
    }
}

/// Failure of an SDK-owned adapter-event handler task.
#[derive(Debug, ThisError)]
pub enum EventError {
    /// The event handler was cancelled or panicked.
    #[error("adapter event handler failed: {0}")]
    Join(tokio::task::JoinError),
}

/// Session startup or teardown failure.
#[derive(Debug, ThisError)]
pub enum Error<E>
where
    E: StdError + 'static,
{
    /// No Tokio runtime was available to schedule the canonical engine.
    #[error("unable to start canonical engine: {0}")]
    EngineStart(engine::SpawnError),
    /// The adapter failed while the engine owner exited normally.
    #[error("adapter runtime failed: {0}")]
    Adapter(E),
    /// The typed event handler failed while other runtime owners exited normally.
    #[error(transparent)]
    Events(EventError),
    /// The engine task failed while the adapter completed normally.
    #[error("engine owner failed: {0}")]
    Engine(engine::TaskError),
    /// Adapter and event-handler ownership both failed during teardown.
    #[error("adapter runtime failed ({adapter}) and event handler failed ({events})")]
    AdapterAndEvents {
        /// Adapter startup or teardown failure.
        adapter: E,
        /// Event-handler task failure.
        events: EventError,
    },
    /// Adapter and engine ownership both failed during the same boundary.
    #[error("adapter runtime failed ({adapter}) and engine owner failed ({engine})")]
    AdapterAndEngine {
        /// Adapter startup or teardown failure.
        adapter: E,
        /// Engine owner join failure.
        engine: engine::TaskError,
    },
    /// Event-handler and engine ownership both failed during teardown.
    #[error("event handler failed ({events}) and engine owner failed ({engine})")]
    EventsAndEngine {
        /// Event-handler task failure.
        events: EventError,
        /// Engine owner join failure.
        engine: engine::TaskError,
    },
    /// Adapter, event-handler, and engine ownership all failed during teardown.
    #[error(
        "adapter runtime failed ({adapter}), event handler failed ({events}), and engine owner failed ({engine})"
    )]
    AdapterEventsAndEngine {
        /// Adapter startup or teardown failure.
        adapter: E,
        /// Event-handler task failure.
        events: EventError,
        /// Engine owner join failure.
        engine: engine::TaskError,
    },
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

    use viperzoo_adapter_api::{action::MockClient, observation, runtime::MockDriver};
    use viperzoo_world::revision::Revision;

    use super::*;

    #[derive(Debug)]
    struct Fake {
        client: MockClient,
        driver: MockDriver,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Event {
        Started,
    }

    impl Adapter for Fake {
        type Client = MockClient;
        type Driver = MockDriver;
        type Error = Infallible;
        type Event = Event;
        type Events = tokio_stream::Iter<std::array::IntoIter<Event, 1>>;

        async fn start<S>(
            self,
            sink: S,
        ) -> Result<viperzoo_adapter_api::runtime::Started<Self>, Self::Error>
        where
            S: observation::Sink + Clone,
        {
            sink.observe(observation::Observation::SessionStarted)
                .await
                .expect("engine accepts the session boundary");

            Ok(AdapterRunning::new(
                self.client,
                tokio_stream::iter([Event::Started]),
                self.driver,
            ))
        }
    }

    fn fake() -> Fake {
        let mut driver = MockDriver::new();
        driver
            .expect_shutdown()
            .times(1)
            .returning(|| Box::pin(std::future::ready(Ok(()))));

        Fake {
            client: MockClient::new(),
            driver,
        }
    }

    #[tokio::test]
    async fn session_owns_adapter_and_engine_as_one_lifecycle() {
        let session = session()
            .engine(engine::Config::default())
            .adapter(fake())
            .start()
            .await
            .expect("fake session starts");

        assert_eq!(session.world.latest().revision(), Revision::INITIAL.next());
        session.owner.shutdown().await.expect("session stops");
    }

    #[tokio::test]
    async fn handled_session_owns_exhaustive_event_delivery() {
        let handled = Arc::new(AtomicUsize::new(0));
        let observations = Arc::clone(&handled);
        let session = session()
            .adapter(fake())
            .on_event(move |event| match event {
                Event::Started => {
                    observations.fetch_add(1, Ordering::Relaxed);
                }
            })
            .start()
            .await
            .expect("handled session starts");

        session.owner.shutdown().await.expect("session stops");

        assert_eq!(handled.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn handled_session_surfaces_event_handler_failure() {
        let session = session()
            .adapter(fake())
            .on_event(|Event::Started| panic!("fixture handler failure"))
            .start()
            .await
            .expect("handled session starts");

        let error = session
            .owner
            .shutdown()
            .await
            .expect_err("handler panic crosses the owner boundary");

        assert!(matches!(error, Error::Events(_)));
    }
}
