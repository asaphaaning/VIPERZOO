//! Asynchronous file-follow acquisition loop.

use std::{io::SeekFrom, path::PathBuf, time::Duration};

use thiserror::Error;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncSeekExt, BufReader, BufWriter},
    time,
};
use tracing::{debug, instrument};
use viperzoo_adapter_api::observation::Observation;
use viperzoo_capture::{
    line::Line,
    record::{self, Record},
};
use viperzoo_engine::{self, Handle};
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
    let mut reader = BufReader::new(file);

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
    let (engine, owner) = viperzoo_engine::channel(viperzoo_engine::Config::default());
    let _owner = tokio::spawn(owner.run());
    let mut phase = Phase::CatchingUp;
    let mut pending = String::new();
    let mut line = Line::FIRST;

    loop {
        let bytes_read = reader
            .read_line(&mut pending)
            .await
            .map_err(|source| Error::Read {
                path: config.capture().to_owned(),
                line,
                source,
            })?;

        if bytes_read == 0 {
            if pending.is_empty() && phase == Phase::CatchingUp {
                phase = Phase::Following;
                let snapshot = engine.snapshot();
                publish_ready(&mut output, config.output(), &snapshot).await?;
                tracing::info!(
                    revision = snapshot.revision().value(),
                    "live projection ready"
                );
            }

            time::sleep(POLL_INTERVAL).await;
            continue;
        }

        if !pending.ends_with('\n') {
            continue;
        }

        let input = pending.trim_end_matches(['\r', '\n']);
        publish_record(
            &mut output,
            config.output(),
            &engine,
            phase,
            record::decode(line, input),
        )
        .await?;

        pending.clear();
        line = line.next().ok_or(Error::LineCapacity)?;
    }
}

#[instrument(
    name = "viperzoo::live::publish_record",
    skip(output, engine, record),
    fields(phase = ?phase, detail = ?detail),
    err,
    ret(level = "debug")
)]
async fn publish_record(
    output: &mut BufWriter<tokio::io::Stdout>,
    detail: Output,
    engine: &Handle,
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
    let change = engine.observe(observation).await?.change();

    match (phase, change) {
        (Phase::Following, Change::Projected(revision)) => {
            let snapshot = engine.snapshot();

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
        line: Line,
        /// Filesystem failure.
        source: std::io::Error,
    },
    /// The stream exceeded the complete `u64` line identity space.
    #[error("live capture exceeds the supported line identity space")]
    LineCapacity,
    /// A typed engine event could not be published.
    #[error(transparent)]
    Message(#[from] message::Error),
    /// The canonical projection owner stopped unexpectedly.
    #[error(transparent)]
    Engine(#[from] viperzoo_engine::Error),
}
