//! Own one adapter and its canonical world as a coherent runtime.
//!
//! [`Builder`] is a lazy value: configuring it has no effect until
//! [`Builder::start`] is awaited. A successful start returns the independent
//! application capabilities in one named [`Session`]. The session itself is
//! the single teardown authority and always joins every selected facility.
//!
//! ```text
//! session() + Adapter + engine::Config
//!          │
//!     Builder::start
//!          │
//!          ├─ actions      ──► dispatch + canonical confirmation
//!          ├─ events       ──► adapter diagnostics
//!          ├─ world        ──► queries
//!          ├─ assets       ──► optional static client knowledge
//!          ├─ capabilities ──► active adapter facilities
//!          └─ lifecycle    ──► wait | shutdown ──► Report
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

use std::{error::Error as StdError, net::SocketAddr, time::Duration};

use thiserror::Error as ThisError;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tracing::{Instrument, instrument};
use viperzoo_adapter_api::runtime::{Adapter, Capabilities, Driver, Running as AdapterRunning};
use viperzoo_assets::Catalog;
use viperzoo_engine as engine;
use viperzoo_world::revision::Revision;

use crate::{action::Actions, diagnostics};

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
    assets: Option<Catalog>,
    diagnostics: Option<diagnostics::Console>,
}

impl Builder {
    fn new() -> Self {
        Self {
            adapter: (),
            engine: engine::Config::default(),
            assets: None,
            diagnostics: None,
        }
    }

    /// Selects the adapter that will acquire observations and perform actions.
    pub fn adapter<A>(self, adapter: A) -> Builder<A> {
        Builder {
            adapter,
            engine: self.engine,
            assets: self.assets,
            diagnostics: self.diagnostics,
        }
    }
}

impl<A> Builder<A> {
    /// Selects the canonical engine configuration.
    pub const fn engine(mut self, engine: engine::Config) -> Self {
        self.engine = engine;
        self
    }

    /// Supplies static client assets to session facilities and applications.
    pub fn assets(mut self, assets: Catalog) -> Self {
        self.assets = Some(assets);
        self
    }

    /// Loads the default client asset catalog into this builder.
    ///
    /// # Errors
    ///
    /// Returns [`viperzoo_assets::LoadError`] when the installed client assets
    /// cannot be located or decoded.
    pub fn load_assets(self) -> Result<Self, viperzoo_assets::LoadError> {
        Ok(self.assets(viperzoo_assets::load_default()?))
    }

    /// Adds an SDK-owned browser diagnostic console.
    pub fn diagnostics(mut self, diagnostics: diagnostics::Console) -> Self {
        self.diagnostics = Some(diagnostics);
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
    /// [`HandledSession`] has no event stream because the session now owns
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

    /// Supplies static client assets to session facilities and applications.
    pub fn assets(mut self, assets: Catalog) -> Self {
        self.builder = self.builder.assets(assets);
        self
    }

    /// Loads the default client asset catalog into this builder.
    ///
    /// # Errors
    ///
    /// Returns [`viperzoo_assets::LoadError`] when the installed client assets
    /// cannot be located or decoded.
    pub fn load_assets(mut self) -> Result<Self, viperzoo_assets::LoadError> {
        self.builder = self.builder.load_assets()?;
        Ok(self)
    }

    /// Adds an SDK-owned browser diagnostic console.
    pub fn diagnostics(mut self, diagnostics: diagnostics::Console) -> Self {
        self.builder = self.builder.diagnostics(diagnostics);
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
            assets,
            capabilities,
            diagnostics,
        } = start(self).await?;

        Ok(Session {
            actions: Actions::new(client, world.clone()),
            events,
            world: world.clone(),
            assets,
            capabilities,
            owner: Owner::new(driver, engine, None, diagnostics, world, capabilities),
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
            assets,
            capabilities,
            diagnostics,
        } = start(self.builder).await?;
        let events = EventTask::spawn(events, self.handler);

        Ok(HandledSession {
            actions: Actions::new(client, world.clone()),
            world: world.clone(),
            assets,
            capabilities,
            owner: Owner::new(
                driver,
                engine,
                Some(events),
                diagnostics,
                world,
                capabilities,
            ),
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
    assets: Option<Catalog>,
    capabilities: Capabilities,
    diagnostics: Option<viperzoo_web::Server>,
}

async fn start<A>(builder: Builder<A>) -> Result<Started<A>, Error<A::Error>>
where
    A: Adapter,
    A::Error: StdError + Send + Sync + 'static,
{
    let capabilities = builder.adapter.capabilities();
    let assets = builder.assets;
    let configured_diagnostics = builder.diagnostics;
    let engine = engine::channel(builder.engine)
        .spawn()
        .map_err(Error::EngineStart)?;
    let ingress = engine.ingress();
    let world = engine.world();
    let engine = engine.owner();
    let diagnostics = match configured_diagnostics {
        Some(diagnostics) => match diagnostics.start(world.clone(), assets.clone()).await {
            Ok(diagnostics) => Some(diagnostics),
            Err(diagnostics) => {
                drop(ingress);
                let engine = engine.wait().await.err();

                return Err(Error::DiagnosticsStart {
                    diagnostics,
                    engine,
                });
            }
        },
        None => None,
    };

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
            assets,
            capabilities,
            diagnostics,
        }),
        Err(adapter) => {
            let diagnostics = shutdown_diagnostics(diagnostics).await.err();
            let engine = engine.wait().await.err();

            Err(Error::AdapterStart {
                adapter,
                diagnostics,
                engine,
            })
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
    actions: Actions<A::Client>,
    events: A::Events,
    world: engine::World,
    assets: Option<Catalog>,
    capabilities: Capabilities,
    owner: Owner<A::Driver>,
}

impl<A> Session<A>
where
    A: Adapter,
{
    /// Borrows typed actions paired with canonical world confirmation.
    #[must_use]
    pub const fn actions(&self) -> &Actions<A::Client> {
        &self.actions
    }

    /// Borrows the adapter-specific lifecycle and diagnostic event stream.
    #[must_use]
    pub fn events(&mut self) -> &mut A::Events {
        &mut self.events
    }

    /// Borrows cloneable read access to the canonical world.
    #[must_use]
    pub const fn world(&self) -> &engine::World {
        &self.world
    }

    /// Borrows static client assets when selected on the builder.
    #[must_use]
    pub const fn assets(&self) -> Option<&Catalog> {
        self.assets.as_ref()
    }

    /// Returns the facilities active on the configured adapter.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Returns the bound diagnostic address when a console is active.
    #[must_use]
    pub fn diagnostics_address(&self) -> Option<SocketAddr> {
        self.owner.diagnostics_address()
    }
}

impl<A> Session<A>
where
    A: Adapter,
    A::Driver: Driver,
    <A::Driver as Driver>::Error: StdError + Send + Sync + 'static,
{
    /// Returns whether any owned runtime authority has stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.owner.is_finished()
    }

    /// Waits until any owned runtime authority stops.
    pub async fn finished(&self) {
        self.owner.finished().await;
    }

    /// Waits for natural termination and reports the completed session shape.
    pub async fn wait(self) -> Result<Report, Error<<A::Driver as Driver>::Error>> {
        self.owner.wait().await
    }

    /// Requests termination and reports the completed session shape.
    pub async fn shutdown(self) -> Result<Report, Error<<A::Driver as Driver>::Error>> {
        self.owner.shutdown().await
    }
}

/// Consumer capabilities of a session whose events are handled by the SDK.
#[derive(Debug)]
#[must_use = "a live handled session must retain its owner until teardown"]
pub struct HandledSession<A>
where
    A: Adapter,
{
    actions: Actions<A::Client>,
    world: engine::World,
    assets: Option<Catalog>,
    capabilities: Capabilities,
    owner: Owner<A::Driver>,
}

impl<A> HandledSession<A>
where
    A: Adapter,
{
    /// Borrows typed actions paired with canonical world confirmation.
    #[must_use]
    pub const fn actions(&self) -> &Actions<A::Client> {
        &self.actions
    }

    /// Borrows cloneable read access to the canonical world.
    #[must_use]
    pub const fn world(&self) -> &engine::World {
        &self.world
    }

    /// Borrows static client assets when selected on the builder.
    #[must_use]
    pub const fn assets(&self) -> Option<&Catalog> {
        self.assets.as_ref()
    }

    /// Returns the facilities active on the configured adapter.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Returns the bound diagnostic address when a console is active.
    #[must_use]
    pub fn diagnostics_address(&self) -> Option<SocketAddr> {
        self.owner.diagnostics_address()
    }
}

impl<A> HandledSession<A>
where
    A: Adapter,
    A::Driver: Driver,
    <A::Driver as Driver>::Error: StdError + Send + Sync + 'static,
{
    /// Returns whether any owned runtime authority has stopped.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.owner.is_finished()
    }

    /// Waits until any owned runtime authority stops.
    pub async fn finished(&self) {
        self.owner.finished().await;
    }

    /// Waits for natural termination and reports the completed session shape.
    pub async fn wait(self) -> Result<Report, Error<<A::Driver as Driver>::Error>> {
        self.owner.wait().await
    }

    /// Requests termination and reports the completed session shape.
    pub async fn shutdown(self) -> Result<Report, Error<<A::Driver as Driver>::Error>> {
        self.owner.shutdown().await
    }
}

/// Runtime authorities hidden behind a [`Session`].
#[derive(Debug)]
#[must_use = "the session owner must be awaited to observe teardown"]
struct Owner<D> {
    driver: D,
    engine: engine::Task,
    events: Option<EventTask>,
    diagnostics: Option<viperzoo_web::Server>,
    world: engine::World,
    capabilities: Capabilities,
}

impl<D> Owner<D> {
    const fn new(
        driver: D,
        engine: engine::Task,
        events: Option<EventTask>,
        diagnostics: Option<viperzoo_web::Server>,
        world: engine::World,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            driver,
            engine,
            events,
            diagnostics,
            world,
            capabilities,
        }
    }

    fn diagnostics_address(&self) -> Option<SocketAddr> {
        self.diagnostics.as_ref().map(viperzoo_web::Server::address)
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

    fn is_finished(&self) -> bool {
        self.handle.is_finished()
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
    fn is_finished(&self) -> bool {
        self.driver.is_finished()
            || self.engine.is_finished()
            || self.events.as_ref().is_some_and(EventTask::is_finished)
            || self
                .diagnostics
                .as_ref()
                .is_some_and(viperzoo_web::Server::is_finished)
    }

    /// Waits until the adapter or canonical engine has finished.
    ///
    /// This borrows the owner so it can be selected alongside application
    /// work. Call [`Session::wait`] or [`Session::shutdown`] afterward to
    /// consume ownership and surface the final lifecycle result.
    #[instrument(name = "viperzoo::sdk::session::finished", skip(self))]
    async fn finished(&self) {
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
    async fn wait(self) -> Result<Report, Error<D::Error>> {
        self.finished().await;
        let adapter_finished = self.driver.is_finished();
        let Self {
            driver,
            engine,
            events,
            diagnostics,
            world,
            capabilities,
        } = self;
        let events_handled = events.is_some();
        let diagnostics_address = diagnostics.as_ref().map(viperzoo_web::Server::address);
        let adapter = if adapter_finished {
            driver.wait().await
        } else {
            driver.shutdown().await
        };
        let events = wait_events(events).await;
        let diagnostics = shutdown_diagnostics(diagnostics).await;
        let engine = engine.wait().await;

        finish(
            Report {
                termination: Termination::Natural,
                revision: world.latest().revision(),
                capabilities,
                events_handled,
                diagnostics_address,
            },
            Outcomes {
                adapter,
                events,
                diagnostics,
                engine,
            },
        )
    }

    /// Requests adapter termination, then joins the engine owner.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if adapter teardown, typed event delivery, or the
    /// engine task fails. Combined failures retain every failed authority.
    #[instrument(name = "viperzoo::sdk::session::shutdown", skip(self), err)]
    async fn shutdown(self) -> Result<Report, Error<D::Error>> {
        let Self {
            driver,
            engine,
            events,
            diagnostics,
            world,
            capabilities,
        } = self;
        let events_handled = events.is_some();
        let diagnostics_address = diagnostics.as_ref().map(viperzoo_web::Server::address);
        let adapter = driver.shutdown().await;
        let events = wait_events(events).await;
        let diagnostics = shutdown_diagnostics(diagnostics).await;
        let engine = engine.wait().await;

        finish(
            Report {
                termination: Termination::Requested,
                revision: world.latest().revision(),
                capabilities,
                events_handled,
                diagnostics_address,
            },
            Outcomes {
                adapter,
                events,
                diagnostics,
                engine,
            },
        )
    }
}

async fn wait_events(events: Option<EventTask>) -> Result<(), EventError> {
    match events {
        Some(events) => events.wait().await,
        None => Ok(()),
    }
}

async fn shutdown_diagnostics(
    diagnostics: Option<viperzoo_web::Server>,
) -> Result<(), viperzoo_web::Error> {
    match diagnostics {
        Some(diagnostics) => diagnostics.shutdown().await,
        None => Ok(()),
    }
}

struct Outcomes<E> {
    adapter: Result<(), E>,
    events: Result<(), EventError>,
    diagnostics: Result<(), viperzoo_web::Error>,
    engine: Result<(), engine::TaskError>,
}

fn finish<E>(report: Report, outcomes: Outcomes<E>) -> Result<Report, Error<E>>
where
    E: StdError + 'static,
{
    if outcomes.adapter.is_ok()
        && outcomes.events.is_ok()
        && outcomes.diagnostics.is_ok()
        && outcomes.engine.is_ok()
    {
        Ok(report)
    } else {
        Err(Error::Teardown(Failure {
            termination: report.termination,
            revision: report.revision,
            adapter: outcomes.adapter.err(),
            events: outcomes.events.err(),
            diagnostics: outcomes.diagnostics.err(),
            engine: outcomes.engine.err(),
        }))
    }
}

/// How a completed session was asked to terminate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Termination {
    /// The adapter stopped without an SDK shutdown request.
    Natural,
    /// The application explicitly requested coordinated shutdown.
    Requested,
}

impl Termination {
    /// Every termination mode in declaration order.
    pub const VARIANTS: [Self; 2] = [Self::Natural, Self::Requested];
}

/// Successful completion of every authority owned by a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Report {
    termination: Termination,
    revision: Revision,
    capabilities: Capabilities,
    events_handled: bool,
    diagnostics_address: Option<SocketAddr>,
}

impl Report {
    /// Returns how termination began.
    #[must_use]
    pub const fn termination(self) -> Termination {
        self.termination
    }

    /// Returns the final canonical world revision.
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    /// Returns the adapter facilities active during the session.
    #[must_use]
    pub const fn capabilities(self) -> Capabilities {
        self.capabilities
    }

    /// Returns whether typed adapter events were owned by an SDK handler.
    #[must_use]
    pub const fn events_handled(self) -> bool {
        self.events_handled
    }

    /// Returns the bound diagnostic address when a console was configured.
    #[must_use]
    pub const fn diagnostics_address(self) -> Option<SocketAddr> {
        self.diagnostics_address
    }
}

/// Every authority failure observed during one coordinated teardown.
#[derive(Debug, ThisError)]
#[error("one or more session authorities failed during {termination:?} teardown")]
pub struct Failure<E>
where
    E: StdError + 'static,
{
    termination: Termination,
    revision: Revision,
    adapter: Option<E>,
    events: Option<EventError>,
    diagnostics: Option<viperzoo_web::Error>,
    engine: Option<engine::TaskError>,
}

impl<E> Failure<E>
where
    E: StdError + 'static,
{
    /// Returns how teardown began.
    #[must_use]
    pub const fn termination(&self) -> Termination {
        self.termination
    }

    /// Returns the final coherent world revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Borrows the adapter failure, when present.
    #[must_use]
    pub const fn adapter(&self) -> Option<&E> {
        self.adapter.as_ref()
    }

    /// Borrows the SDK event-handler failure, when present.
    #[must_use]
    pub const fn events(&self) -> Option<&EventError> {
        self.events.as_ref()
    }

    /// Borrows the diagnostic-console failure, when present.
    #[must_use]
    pub const fn diagnostics(&self) -> Option<&viperzoo_web::Error> {
        self.diagnostics.as_ref()
    }

    /// Borrows the canonical engine failure, when present.
    #[must_use]
    pub const fn engine(&self) -> Option<&engine::TaskError> {
        self.engine.as_ref()
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
    /// The diagnostic console could not start; cleanup may also have failed.
    #[error("diagnostic console could not start: {diagnostics}")]
    DiagnosticsStart {
        /// Console bind or startup failure.
        diagnostics: viperzoo_web::Error,
        /// Engine cleanup failure after the rejected start.
        engine: Option<engine::TaskError>,
    },
    /// The adapter could not start; cleanup retains any additional failures.
    #[error("adapter could not start: {adapter}")]
    AdapterStart {
        /// Adapter startup failure.
        adapter: E,
        /// Diagnostic shutdown failure during cleanup.
        diagnostics: Option<viperzoo_web::Error>,
        /// Engine cleanup failure during cleanup.
        engine: Option<engine::TaskError>,
    },
    /// Coordinated teardown observed one or more authority failures.
    #[error(transparent)]
    Teardown(#[from] Failure<E>),
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

    use viperzoo_adapter_api::{
        action::MockClient,
        observation,
        runtime::{Capabilities, Capability, MockDriver},
    };
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

        fn capabilities(&self) -> Capabilities {
            Capabilities::new(&[Capability::Actions, Capability::Events])
        }

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

    fn naturally_stopping_fake() -> Fake {
        let mut driver = MockDriver::new();
        driver.expect_is_finished().returning(|| false);
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

        assert_eq!(
            session.world().latest().revision(),
            Revision::INITIAL.next()
        );
        assert!(!session.capabilities().contains(Capability::WarmAttachment));

        let report = session.shutdown().await.expect("session stops");

        assert_eq!(report.termination(), Termination::Requested);
        assert_eq!(report.revision(), Revision::INITIAL.next());
        assert!(!report.events_handled());
    }

    #[tokio::test]
    async fn natural_facility_completion_coordinates_the_remaining_authorities() {
        let session = session()
            .adapter(naturally_stopping_fake())
            .on_event(drop)
            .start()
            .await
            .expect("session starts");
        let report = session.wait().await.expect("session stops");

        assert_eq!(report.termination(), Termination::Natural);
        assert!(report.events_handled());
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

        session.shutdown().await.expect("session stops");

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
            .shutdown()
            .await
            .expect_err("handler panic crosses the owner boundary");

        assert!(matches!(
            error,
            Error::Teardown(ref failure) if failure.events().is_some()
        ));
    }
}
