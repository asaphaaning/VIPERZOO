//! Example VIPERZOO script that walks to a coordinate on the current map.

mod cli;

use std::process::ExitCode;

use thiserror::Error;
use tracing::{info, instrument};
use viperzoo_adapter_frida as frida;
use viperzoo_sdk::{actions, assets, engine, protocol::primitive::Position};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let config = match cli::Config::parse(std::env::args_os().skip(1)) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "walk configuration rejected");
            return ExitCode::FAILURE;
        }
    };

    match run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "destination walk failed");
            ExitCode::FAILURE
        }
    }
}

#[instrument(
    name = "viperzoo::app::walk::run",
    skip(config),
    fields(x = config.x(), y = config.y()),
    err,
    ret(level = "debug")
)]
async fn run(config: cli::Config) -> Result<(), Error> {
    let channel = engine::channel(engine::Config::default());
    let ingress = channel.ingress();
    let world = channel.world();
    let owner = tokio::spawn(channel.owner().run());
    let running = frida::attach(
        frida::Config::new(config.client().clone()).with_recording(config.recording().clone()),
        ingress.clone(),
    )?;
    let control = running.client;
    let _events = running.events;
    let driver = running.driver;
    let target = Position::new(config.x(), config.y());
    let assets = assets::load_default()?;
    info!(
        definitions = assets.len(),
        source = %assets.source().display(),
        "static object collision catalog loaded"
    );
    let result = actions::walk::to_with_assets(
        &control,
        &world,
        &assets,
        target,
        actions::walk::Config::default(),
    )
    .await;

    let adapter = driver.shutdown().await;
    drop(ingress);
    let owner = owner.await;
    let report = result?;

    adapter?;
    owner?;

    info!(
        x = report.target().x().value(),
        y = report.target().y().value(),
        attempts = report.attempts(),
        learned_edges = report.learned_edges(),
        "destination reached"
    );

    Ok(())
}

/// Fatal example-script failure.
#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Frida(#[from] frida::Error),
    #[error(transparent)]
    Walk(#[from] actions::walk::Error<frida::ActionError>),
    #[error("engine owner failed: {0}")]
    Owner(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Assets(#[from] assets::LoadError),
}
