//! Differentially verifies Rust projection against the Python reference state.

mod cli;
mod reference;
mod verification;

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
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            tracing::error!(error = %error, "differential verification failed to run");
            ExitCode::FAILURE
        }
    }
}

#[instrument(name = "viperzoo::app::verify::run", err, ret(level = "debug"))]
fn run() -> Result<bool, Error> {
    let config = cli::Config::parse(std::env::args_os().skip(1))?;
    let capture_file = File::open(config.capture()).map_err(|source| Error::Open {
        path: config.capture().to_owned(),
        source,
    })?;
    let replay = capture::replay(BufReader::new(capture_file), config.capture())?;
    let reference = reference::State::read(config.reference())?;
    let report = verification::Report::compare(&replay, &reference);

    println!("{}", serde_json::to_string_pretty(&report)?);

    if report.passed() {
        tracing::info!("Rust and Python projections match on every claimed field");
    } else {
        tracing::error!("Rust and Python projections diverge on one or more claimed fields");
    }

    Ok(report.passed())
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
    #[error(transparent)]
    Reference(#[from] reference::Error),
    #[error("unable to serialize verification report: {0}")]
    Serialize(#[from] serde_json::Error),
}
