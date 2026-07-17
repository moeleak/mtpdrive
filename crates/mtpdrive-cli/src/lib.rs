//! Command-line interface for `MTPDrive`.

mod args;
mod commands;
mod output;

use anyhow::Result;
use clap::Parser;

/// Parses command-line arguments and executes the selected command.
///
/// # Errors
///
/// Returns an error when daemon startup, IPC, serialization, or output fails.
pub async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mtpdrive=info".into()),
        )
        .with_target(false)
        .init();

    let cli = args::Cli::parse();
    let stdout = std::io::stdout();
    commands::execute(cli, &mut stdout.lock()).await
}
