//! Project the live engine into a browser as a read-only diagnostic console.
//!
//! An application that already owns an [`engine::World`] can start this
//! alongside its normal work and then watch the same world state, and the same
//! tracing output, from any machine on the network.
//!
//! ```text
//!  engine::World ──subscribe──► snapshot ──project──► Event::World ─┐
//!                                                                    ├─► /events ─► browser
//!  tracing fmt::layer(stdout) ── web::trace::Layer ──► Event::Trace ─┘
//!                                                                    └─► /  (console page)
//! ```
//!
//! The stream is one-directional by construction, which is why it is
//! server-sent events rather than a socket: [`event::Event`] has no command
//! variants, so a console can observe the engine but never drive it. Browsers
//! also reconnect an `EventSource` on their own after a restart.
//!
//! # Example
//!
//! ```no_run
//! # async fn example(world: viperzoo_engine::World) -> Result<(), viperzoo_web::Error> {
//! use tracing_subscriber::prelude::*;
//!
//! let console = viperzoo_web::Console::new();
//!
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::fmt::layer())
//!     .with(console.layer())
//!     .init();
//!
//! console.serve(world, None, "0.0.0.0:7878".parse().unwrap(), None).await
//! # }
//! ```

pub mod control;
pub mod event;
pub mod projection;
pub mod trace;

use std::{
    collections::VecDeque,
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{
        Html, IntoResponse,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt as _, wrappers::BroadcastStream};
use tracing::{debug, info, instrument};
use viperzoo_assets::Catalog;
use viperzoo_engine::World as EngineWorld;

use crate::{event::Event, projection::World};

/// Console page, embedded so a running engine needs no asset directory.
const PAGE: &str = include_str!("console.html");

/// How many events a slow console may fall behind before it is dropped.
const BACKLOG: usize = 512;

/// Tracing lines retained so a console that connects mid-session sees history.
///
/// Without this a fresh page shows an empty log until the next event, which
/// for a workflow that speaks every few minutes reads as a broken feed.
const HISTORY: usize = 300;

/// Smallest gap between world projections sent to one console.
///
/// The engine revises state on every decoded packet — several times a second,
/// each carrying the full tile and entity set. Nobody reads faster than this,
/// and coalescing keeps the socket from drowning tracing lines in re-sends.
const WORLD_INTERVAL: Duration = Duration::from_millis(250);

/// A console broadcast plus the routes that serve it.
///
/// Constructing a `Console` allocates the shared channel only. Nothing is
/// observed until [`Console::layer`] is installed or [`Console::serve`] is
/// awaited, so a console that is created and dropped costs nothing.
#[derive(Clone, Debug)]
#[must_use = "a console does nothing until `serve` is awaited"]
pub struct Console {
    events: broadcast::Sender<Event>,
    history: Arc<Mutex<VecDeque<event::Trace>>>,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    /// Creates a console with an empty event channel.
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(BACKLOG);

        Self {
            events,
            history: Arc::new(Mutex::new(VecDeque::with_capacity(HISTORY))),
        }
    }

    /// Returns the tracing layer that mirrors terminal output to this console.
    ///
    /// Compose it alongside `fmt::layer()` rather than in place of it.
    #[must_use]
    pub fn layer(&self) -> trace::Layer {
        trace::Layer::new(self.events.clone(), Arc::clone(&self.history))
    }

    /// Serves the console until the process ends.
    ///
    /// Publishes a fresh [`projection::World`] on every engine revision and
    /// forwards tracing events as they arrive. `assets` supplies static
    /// fixture collision for the tile inspector; without it that field is
    /// simply absent.
    ///
    /// `controls` is what keeps this crate from assuming an application exists
    /// to be driven. Without it the page renders no controls and the signal
    /// route reports that nothing is listening, so a read-only consumer such as
    /// a replay viewer is unaffected by the surface existing at all.
    ///
    /// # Errors
    ///
    /// Returns [`enum@Error`] when the address cannot be bound or the HTTP
    /// server fails.
    #[instrument(
        name = "viperzoo::web::serve",
        skip(self, world, assets, controls),
        err
    )]
    pub async fn serve(
        &self,
        world: EngineWorld,
        assets: Option<Catalog>,
        address: SocketAddr,
        controls: Option<control::Controls>,
    ) -> Result<(), Error> {
        let state = Shared {
            events: self.events.clone(),
            history: Arc::clone(&self.history),
            world: world.clone(),
            assets: assets.clone(),
            controls,
        };

        // One publisher serves every console: projecting once per interval and
        // broadcasting keeps a second viewer from doubling the work, and keeps
        // world frames from crowding out tracing on a busy session.
        tokio::spawn(publish(
            world,
            assets,
            self.events.clone(),
            state.controls.clone(),
        ));

        let router = Router::new()
            .route("/", get(async || Html(PAGE)))
            .route("/events", get(events))
            // Control is a separate, one-way-in surface. The event stream stays
            // exactly as it was: `event::Event` still has no command variants,
            // so what a console reads can never drive anything.
            .route("/signal", post(signal))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|source| Error::Bind { address, source })?;

        info!(%address, "console available");
        axum::serve(listener, router).await.map_err(Error::Serve)
    }
}

/// Shared handles every console request needs.
#[derive(Clone)]
struct Shared {
    events: broadcast::Sender<Event>,
    history: Arc<Mutex<VecDeque<event::Trace>>>,
    world: EngineWorld,
    assets: Option<Catalog>,
    controls: Option<control::Controls>,
}

/// Accepts one control request and answers with the state it produced.
///
/// The reply carries what the application *is*, not what was asked, so a console
/// cannot paint itself paused before the application has actually held.
async fn signal(
    State(shared): State<Shared>,
    Json(signal): Json<control::Signal>,
) -> impl IntoResponse {
    let Some(controls) = shared.controls else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(control::State::Stopped),
        );
    };

    if controls.request(signal).await {
        (StatusCode::ACCEPTED, Json(controls.state()))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(control::State::Stopped),
        )
    }
}

/// Re-projects the world at a bounded rate for as long as the engine runs.
///
/// The engine revises state on every decoded packet, several times a second,
/// each carrying the full tile and entity set. Nobody reads that fast, so
/// revisions are coalesced into one projection per [`WORLD_INTERVAL`].
async fn publish(
    world: EngineWorld,
    assets: Option<Catalog>,
    events: broadcast::Sender<Event>,
    controls: Option<control::Controls>,
) {
    let mut snapshots = world.subscribe();

    loop {
        let snapshot = match snapshots.changed().await {
            Ok(snapshot) => snapshot,
            Err(_) => {
                debug!("engine stopped; console projection ended");
                return;
            }
        };

        // The state travels with the world frame so the console never has to
        // infer it from its own last request.
        let world = World::project(
            &snapshot,
            assets.as_ref(),
            controls.as_ref().map(control::Controls::state),
        );

        // No subscriber simply means no console is open.
        let _ = events.send(Event::world(world));
        tokio::time::sleep(WORLD_INTERVAL).await;
    }
}

/// Streams world projections and tracing events to one console.
///
/// A console that connects mid-session receives the current world and the
/// retained tracing lines before the live stream, so it never opens onto an
/// empty panel while the workflow is between events.
async fn events(
    State(shared): State<Shared>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let initial = World::project(
        &shared.world.latest(),
        shared.assets.as_ref(),
        shared.controls.as_ref().map(control::Controls::state),
    );
    let replayed = replay(&shared.history);
    let backlog = std::iter::once(Event::world(initial))
        .chain(replayed.into_iter().map(Event::trace))
        .map(|event| Ok(encode(&event)));
    let live = BroadcastStream::new(shared.events.subscribe())
        .filter_map(|received| received.ok().map(|event| Ok(encode(&event))));

    Sse::new(tokio_stream::iter(backlog).chain(live)).keep_alive(KeepAlive::default())
}

/// Renders one event as a named server-sent event.
///
/// The name lets a browser attach a listener per kind instead of switching on
/// a tag, and a body that cannot serialize degrades to an empty payload rather
/// than tearing down a console's stream.
fn encode(event: &Event) -> SseEvent {
    let name = match event {
        Event::World(_) => "world",
        Event::Trace(_) => "trace",
    };

    SseEvent::default()
        .event(name)
        .data(serde_json::to_string(event).unwrap_or_default())
}

/// Copies retained tracing lines without holding the lock across an await.
fn replay(history: &Arc<Mutex<VecDeque<event::Trace>>>) -> Vec<event::Trace> {
    history
        .lock()
        .map(|history| history.iter().cloned().collect())
        .unwrap_or_default()
}

/// Console hosting failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The console address could not be bound.
    #[error("unable to bind console address {address}: {source}")]
    Bind {
        /// Requested listen address.
        address: SocketAddr,
        /// Underlying socket failure.
        source: std::io::Error,
    },
    /// The HTTP server stopped with an error.
    #[error("console server failed: {0}")]
    Serve(#[source] std::io::Error),
}
