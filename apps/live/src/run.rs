//! Asynchronous file-follow acquisition loop.

use std::{io::SeekFrom, path::PathBuf, time::Duration};

use bytes::BytesMut;
use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, BufWriter},
    time,
};
use tokio_util::codec::Decoder;
use tracing::{debug, instrument};
use viperzoo_adapter_api::observation::Observation;
use viperzoo_capture::{
    codec::{self, Codec},
    line::Line,
    record::Record,
};
use viperzoo_engine::{self, Ingress, World};
use viperzoo_world::world::Change;

use crate::{
    cli::{Config, Output, Start},
    message::{self, Message, Summary},
};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Follows an actively appended capture until the process is cancelled.
#[instrument(
    name = "viperzoo::live::follow",
    skip(config),
    fields(source = %config.capture().display(), start = ?config.start()),
    err,
    ret(level = "debug")
)]
pub async fn follow(config: &Config) -> Result<(), Error> {
    let file = File::open(config.capture())
        .await
        .map_err(|source| Error::Open {
            path: config.capture().to_owned(),
            source,
        })?;
    let mut reader = file;

    if config.start() == Start::End {
        reader
            .seek(SeekFrom::End(0))
            .await
            .map_err(|source| Error::Seek {
                path: config.capture().to_owned(),
                source,
            })?;
    }

    let mut output = BufWriter::new(tokio::io::stdout());
    let channel = viperzoo_engine::channel(viperzoo_engine::Config::default());
    let engine = channel.spawn()?;
    let ingress = engine.ingress();
    let world = engine.world();
    let _engine = engine.owner();
    let mut phase = Phase::CatchingUp;
    let mut pending = BytesMut::new();
    let mut codec = Codec::new();

    loop {
        let bytes_read = reader
            .read_buf(&mut pending)
            .await
            .map_err(|source| Error::Read {
                path: config.capture().to_owned(),
                line: codec.next_line(),
                source,
            })?;

        if bytes_read == 0 {
            if pending.is_empty() && phase == Phase::CatchingUp {
                phase = Phase::Following;
                let snapshot = world.latest();
                publish_ready(&mut output, config.output(), &snapshot).await?;
                tracing::info!(
                    revision = snapshot.revision().value(),
                    "live projection ready"
                );
            }

            time::sleep(POLL_INTERVAL).await;
            continue;
        }

        while let Some(record) = codec.decode(&mut pending)? {
            publish_record(
                &mut output,
                config.output(),
                &ingress,
                &world,
                phase,
                record,
            )
            .await?;
        }
    }
}

#[instrument(
    name = "viperzoo::live::publish_record",
    skip(output, ingress, world, record),
    fields(phase = ?phase, detail = ?detail),
    err,
    ret(level = "debug")
)]
async fn publish_record(
    output: &mut BufWriter<tokio::io::Stdout>,
    detail: Output,
    ingress: &Ingress,
    world: &World,
    phase: Phase,
    record: Record,
) -> Result<(), Error> {
    let observation = match record {
        Record::SessionStarted => Observation::SessionStarted,
        Record::TransportClosed => Observation::TransportClosed,
        Record::Packet(packet) => packet.into(),
        Record::Rejected(diagnostic) => {
            debug!(diagnostic = ?diagnostic, "live capture row quarantined");
            message::write(
                output,
                Message::Diagnostic {
                    diagnostic: &diagnostic,
                },
            )
            .await?;

            return Ok(());
        }
        Record::Skipped => return Ok(()),
    };
    let change = ingress.observe(observation).await?.change();

    match (phase, change) {
        (Phase::Following, Change::Projected(revision)) => {
            let snapshot = world.latest();

            match detail {
                Output::Summary => {
                    message::write(
                        output,
                        Message::ProjectedSummary {
                            summary: Summary::from_snapshot(&snapshot),
                        },
                    )
                    .await?;
                }
                Output::Snapshot => {
                    message::write(
                        output,
                        Message::ProjectedSnapshot {
                            snapshot: &snapshot,
                        },
                    )
                    .await?;
                }
            }

            debug!(revision = revision.value(), "live projection changed");
        }
        (Phase::CatchingUp, change) => {
            debug!(
                revision = change.revision().value(),
                projected = change.is_projected(),
                "historical observation reduced"
            );
        }
        (Phase::Following, Change::Recorded(revision)) => {
            debug!(revision = revision.value(), "live observation recorded");
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    CatchingUp,
    Following,
}

async fn publish_ready(
    output: &mut BufWriter<tokio::io::Stdout>,
    detail: Output,
    snapshot: &viperzoo_world::snapshot::Snapshot,
) -> Result<(), message::Error> {
    match detail {
        Output::Summary => {
            message::write(
                output,
                Message::ReadySummary {
                    summary: Summary::from_snapshot(snapshot),
                },
            )
            .await
        }
        Output::Snapshot => message::write(output, Message::ReadySnapshot { snapshot }).await,
    }
}

/// Fatal live acquisition failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The canonical engine could not be scheduled.
    #[error(transparent)]
    EngineStart(#[from] viperzoo_engine::SpawnError),
    /// The capture file could not be opened.
    #[error("unable to open live capture {path}: {source}")]
    Open {
        /// Capture path.
        path: PathBuf,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// End attachment could not seek to the current boundary.
    #[error("unable to seek live capture {path}: {source}")]
    Seek {
        /// Capture path.
        path: PathBuf,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// The active stream could not be read.
    #[error("unable to read live capture {path} at acquisition line {line:?}: {source}")]
    Read {
        /// Capture path.
        path: PathBuf,
        /// One-based line since the selected attachment position.
        line: Option<Line>,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// Capture row framing failed.
    #[error(transparent)]
    Capture(#[from] codec::Error),
    /// A typed engine event could not be published.
    #[error(transparent)]
    Message(#[from] message::Error),
    /// Canonical observation ingress stopped unexpectedly.
    #[error(transparent)]
    Ingress(#[from] viperzoo_engine::Error),
}
