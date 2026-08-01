//! Standalone direct-attachment VIPERZOO engine.

mod cli;
mod run;

use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let config = match cli::Config::parse(std::env::args_os().skip(1)) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "standalone engine configuration rejected");
            return ExitCode::FAILURE;
        }
    };

    match run::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "standalone engine stopped with an error");
            ExitCode::FAILURE
        }
    }
}
