use crate::args::{Cli, Command};
use crate::output;
use anyhow::{Context, Result, bail};
use mtpdrive_core::{
    AppPaths, DaemonClient, DaemonOptions, DaemonRequestError, Language, MtpDriveDaemon,
    current_language,
};
use std::io::Write;
use std::time::Duration;

pub(crate) async fn execute(cli: Cli, output: &mut dyn Write) -> Result<()> {
    match cli.command.unwrap_or(Command::Status { json: false }) {
        Command::Daemon {
            no_mount,
            port,
            no_open,
        } => {
            let paths = AppPaths::discover()?;
            let options = DaemonOptions {
                no_mount,
                port,
                open_on_first_device: !no_open,
                ..DaemonOptions::default()
            };
            MtpDriveDaemon::new(paths, options)?.run().await?;
        }
        command => run_client_command(command, output).await?,
    }
    Ok(())
}

async fn run_client_command(command: Command, output: &mut dyn Write) -> Result<()> {
    let language = current_language();
    let strings = language.strings();
    let client = DaemonClient::discover()?;
    match command {
        Command::Status { json } => {
            let snapshot = client.snapshot().await.context(strings.service_hint)?;
            if json {
                writeln!(output, "{}", serde_json::to_string_pretty(&snapshot)?)?;
            } else {
                output::status(output, language, &snapshot)?;
            }
        }
        Command::Devices { json } => {
            let devices = typed_response(language, client.devices().await, true)?;
            if json {
                writeln!(output, "{}", serde_json::to_string_pretty(&devices)?)?;
            } else {
                output::devices(output, language, &devices)?;
            }
        }
        Command::Logs {
            follow,
            json,
            limit,
        } => stream_logs(output, language, &client, follow, json, limit).await?,
        Command::Mount => {
            typed_response(language, client.mount().await, false)?;
        }
        Command::Unmount => {
            typed_response(language, client.unmount().await, false)?;
        }
        Command::Open => {
            typed_response(language, client.open_in_finder().await, false)?;
        }
        Command::Refresh => {
            let devices = typed_response(language, client.refresh().await, false)?;
            output::devices(output, language, &devices)?;
        }
        Command::Shutdown => {
            typed_response(language, client.shutdown().await, false)?;
        }
        Command::Daemon { .. } => unreachable!("daemon handled before client dispatch"),
    }
    Ok(())
}

async fn stream_logs(
    output_writer: &mut dyn Write,
    language: Language,
    client: &DaemonClient,
    follow: bool,
    json: bool,
    limit: usize,
) -> Result<()> {
    let mut after = 0_u64;
    loop {
        let records = typed_response(language, client.logs(after, limit).await, true)?;
        for record in &records {
            output::log(output_writer, record, json)?;
            after = after.max(record.id);
        }
        if !follow {
            break;
        }
        output_writer.flush()?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

fn typed_response<T>(
    language: Language,
    result: mtpdrive_core::DaemonRequestResult<T>,
    service_context: bool,
) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(DaemonRequestError::Client(error)) if service_context => {
            Err(error).context(language.strings().service_hint)
        }
        Err(DaemonRequestError::Client(error)) => Err(error.into()),
        Err(DaemonRequestError::Daemon(message)) => bail!(message),
        Err(DaemonRequestError::Unexpected(response)) => {
            bail!(language.unexpected_daemon_response(response))
        }
    }
}
