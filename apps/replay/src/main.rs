//! Replays plaintext JSONL captures into one projected VIPERZOO snapshot.

mod cli;

use std::{fs::File, io::BufReader, path::PathBuf, process::ExitCode};

use thiserror::Error;
use tracing::instrument;
use viperzoo_replay::capture;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "replay failed");
            ExitCode::FAILURE
        }
    }
}

#[instrument(name = "viperzoo::app::replay::run", err, ret(level = "debug"))]
fn run() -> Result<(), Error> {
    let config = cli::Config::parse(std::env::args_os().skip(1))?;
    let file = File::open(config.capture()).map_err(|source| Error::Open {
        path: config.capture().to_owned(),
        source,
    })?;
    let report = capture::replay(BufReader::new(file), config.capture())?;

    tracing::info!(
        revision = report.snapshot().revision().value(),
        diagnostics = report.diagnostics().len(),
        "capture projected"
    );

    let output = match config.output() {
        cli::Output::Compact => serde_json::to_string(&report),
        cli::Output::Pretty => serde_json::to_string_pretty(&report),
    }?;

    println!("{output}");
    Ok(())
}

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Cli(#[from] cli::Error),
    #[error("unable to open capture {path}: {source}")]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Replay(#[from] capture::Error),
    #[error("unable to serialize replay report: {0}")]
    Serialize(#[from] serde_json::Error),
}
