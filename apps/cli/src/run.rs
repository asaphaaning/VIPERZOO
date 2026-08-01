//! Composition of the direct adapter and canonical engine owner.

use std::time::Duration;

use thiserror::Error;
use tokio::time;
use tracing::{info, instrument, warn};
use viperzoo_adapter_frida::{self as frida, Event};
use viperzoo_world::snapshot::Snapshot;

use crate::cli::Config;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Runs direct acquisition until Ctrl+C or client detach.
#[instrument(
    name = "viperzoo::app::standalone::run",
    skip(config),
    err,
    ret(level = "debug")
)]
pub async fn run(config: Config) -> Result<(), Error> {
    let (engine, owner) = viperzoo_engine::channel(viperzoo_engine::Config::default());
    let owner = tokio::spawn(owner.run());
    let adapter_config = frida::Config::new(config.target().clone())
        .with_agent(config.agent().clone())
        .with_recording(config.recording().clone());
    let attachment = frida::attach(adapter_config, engine.clone())?;
    let mut snapshots = engine.subscribe();
    let mut ticker = time::interval(EVENT_POLL_INTERVAL);

    let stop = loop {
        drain_events(&attachment);

        if attachment.is_finished() {
            break Stop::Detached;
        }

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result?;
                break Stop::Interrupted;
            }
            result = snapshots.changed() => {
                result.map_err(|_| Error::EngineStopped)?;
                print_summary(&snapshots.borrow());
            }
            _ = ticker.tick() => {}
        }
    };

    drain_events(&attachment);
    let adapter = match stop {
        Stop::Interrupted => attachment.stop().await,
        Stop::Detached => attachment.wait().await,
    };
    let shutdown = engine.shutdown().await;
    let owner = owner.await;

    adapter?;
    shutdown?;
    owner?;

    Ok(())
}

fn drain_events(attachment: &frida::Attachment) {
    while let Ok(event) = attachment.events().try_recv() {
        match event {
            Event::Attached(info) => info!(
                pid = info.pid(),
                frida = info.frida_version(),
                "direct Frida attachment loaded"
            ),
            Event::Ready(info) => info!(pid = info.pid(), "plaintext packet hooks ready"),
            Event::ResourcesSeeded(resources) => info!(
                vita = resources.vita().current(),
                max_vita = resources.vita().maximum(),
                mana = resources.mana().current(),
                max_mana = resources.mana().maximum(),
                "player resources seeded from client memory"
            ),
            Event::InventorySeeded { capacity, occupied } => info!(
                capacity,
                occupied, "carried inventory seeded from client memory"
            ),
            Event::TransportClosed => {
                warn!("NexusTK game transport closed below the plaintext packet boundary");
            }
            Event::TransportFault(fault) => warn!(
                operation = ?fault.operation(),
                code = fault.code(),
                "NexusTK game transport reported a socket fault"
            ),
            Event::Rejected(rejection) => warn!(
                flow = ?rejection.flow(),
                length = rejection.length(),
                reason = rejection.reason(),
                "Frida callback rejected"
            ),
            Event::Warning(message) => warn!(message = %message, "Frida agent warning"),
            Event::ScriptError(problem) => warn!(
                description = problem.description(),
                stack = problem.stack(),
                "Frida agent failed"
            ),
        }
    }
}

fn print_summary(snapshot: &Snapshot) {
    let map = snapshot.map().context();
    let resources = snapshot.player().resources();
    let position = snapshot.player().location().position();
    let vita = resources.vita();
    let mana = resources.mana();

    info!(
        revision = snapshot.revision().value(),
        packets = snapshot.processed_packet_count(),
        unknown = snapshot.unknown_packet_count(),
        map_id = map.map(|context| context.id().value()),
        map_title = map.map(viperzoo_world::map::Context::title),
        x = position.map(|position| position.x().value()),
        y = position.map(|position| position.y().value()),
        vita = vita.current().value(),
        max_vita = vita.maximum().value(),
        mana = mana.current().value(),
        max_mana = mana.maximum().value(),
        tiles = snapshot.map().tiles().len(),
        entities = snapshot.entities().len(),
        "world projection changed"
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stop {
    Interrupted,
    Detached,
}

/// Fatal standalone engine failure.
#[derive(Debug, Error)]
pub enum Error {
    /// Direct Frida acquisition failed.
    #[error(transparent)]
    Frida(#[from] frida::Error),
    /// The canonical engine stopped unexpectedly.
    #[error(transparent)]
    Engine(#[from] viperzoo_engine::Error),
    /// The canonical snapshot publisher stopped unexpectedly.
    #[error("canonical engine snapshot stream stopped")]
    EngineStopped,
    /// The canonical engine owner panicked.
    #[error("canonical engine owner failed: {0}")]
    Owner(#[from] tokio::task::JoinError),
    /// The terminal signal handler could not be installed.
    #[error("unable to listen for Ctrl+C: {0}")]
    Signal(#[from] std::io::Error),
}
