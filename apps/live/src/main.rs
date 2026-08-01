//! Continuously projects an actively appended Frida JSONL tap.

mod cli;
mod message;
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
            tracing::error!(error = %error, "live projection configuration rejected");
            return ExitCode::FAILURE;
        }
    };

    match run::follow(&config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "live projection failed");
            ExitCode::FAILURE
        }
    }
}
