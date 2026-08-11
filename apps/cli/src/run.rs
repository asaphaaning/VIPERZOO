//! Composition of the direct adapter and canonical engine owner.

use thiserror::Error;
use tracing::{info, instrument, warn};
use viperzoo_adapter_frida::{self as frida, Event};
use viperzoo_sdk::{session, world::snapshot::Snapshot};

use crate::cli::Config;

/// Runs direct acquisition until Ctrl+C or client detach.
#[instrument(
    name = "viperzoo::app::standalone::run",
    skip(config),
    err,
    ret(level = "debug")
)]
pub async fn run(config: Config) -> Result<(), Error> {
    let adapter_config = frida::Config::new(config.target().clone())
        .with_agent(config.agent().clone())
        .with_recording(config.recording().clone());
    let session = session()
        .adapter(frida::Adapter::new(adapter_config))
        .on_event(report_event)
        .start()
        .await?;
    let _actions = session.actions;
    let world = session.world;
    let owner = session.owner;
    let mut snapshots = world.subscribe();

    let stop = loop {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result?;
                break Stop::Interrupted;
            }
            () = owner.finished() => break Stop::Detached,
            result = snapshots.changed() => {
                let snapshot = result.map_err(|_| Error::WorldStopped)?;
                print_summary(&snapshot);
            }
        }
    };

    match stop {
        Stop::Interrupted => owner.shutdown().await?,
        Stop::Detached => owner.wait().await?,
    }

    Ok(())
}

fn report_event(event: Event) {
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
        Event::MapIdentitySeeded(identity) => info!(
            map = identity.id().value(),
            width = identity.dimensions().width(),
            height = identity.dimensions().height(),
            "map identity seeded from client memory"
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
        map_title = map.map(viperzoo_sdk::world::map::Context::title),
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
    /// Session startup or teardown failed.
    #[error(transparent)]
    Session(#[from] viperzoo_sdk::Error<frida::Error>),
    /// The canonical world subscription closed unexpectedly.
    #[error("canonical world subscription closed")]
    WorldStopped,
    /// The terminal signal handler could not be installed.
    #[error("unable to listen for Ctrl+C: {0}")]
    Signal(#[from] std::io::Error),
}
