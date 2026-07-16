use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use mtpdrive_core::{
    AppPaths, ControlRequest, ControlResponse, DaemonClient, DaemonOptions, DeviceSummary,
    LogRecord, MtpDriveDaemon,
};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "mtpdrive",
    version,
    about = "Expose Android MTP devices as a native macOS NFS volume"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the USB, NFS, and control service in the foreground.
    Daemon {
        /// Start NFS without mounting it in Finder.
        #[arg(long)]
        no_mount: bool,
        /// Local TCP port (0 chooses a free port).
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Do not open Finder when the first phone appears.
        #[arg(long)]
        no_open: bool,
    },
    /// Show service and mount status.
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List connected MTP devices and storage areas.
    Devices {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Mount the local NFS volume at ~/MTPDrive.
    Mount,
    /// Unmount the MTPDrive volume.
    Unmount,
    /// Open the mounted volume in Finder.
    Open,
    /// Rescan USB devices now.
    Refresh,
    /// Read structured service logs.
    Logs {
        /// Continue printing new records.
        #[arg(short, long)]
        follow: bool,
        /// Emit one JSON object per line.
        #[arg(long)]
        json: bool,
        /// Maximum initial record count.
        #[arg(short = 'n', long, default_value_t = 500)]
        limit: usize,
    },
    /// Ask the running service to exit cleanly.
    Shutdown,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mtpdrive=info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
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
        command => run_client_command(command).await?,
    }
    Ok(())
}

async fn run_client_command(command: Command) -> Result<()> {
    let client = DaemonClient::discover()?;
    match command {
        Command::Status { json } => {
            let snapshot = client.snapshot().await.context(service_hint())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                println!("MTPDrive {}", snapshot.version);
                println!("Mount: {:?}", snapshot.mount);
                println!("Devices: {}", snapshot.devices.len());
                if let Some(error) = snapshot.last_error {
                    println!("Last error: {error}");
                }
            }
        }
        Command::Devices { json } => {
            let response = client
                .request(ControlRequest::Devices)
                .await
                .context(service_hint())?;
            let ControlResponse::Devices(devices) = response else {
                return response_error(response);
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else {
                print_devices(&devices);
            }
        }
        Command::Logs {
            follow,
            json,
            limit,
        } => stream_logs(&client, follow, json, limit).await?,
        Command::Mount => expect_ok(client.request(ControlRequest::Mount).await?)?,
        Command::Unmount => expect_ok(client.request(ControlRequest::Unmount).await?)?,
        Command::Open => expect_ok(client.request(ControlRequest::Open).await?)?,
        Command::Refresh => {
            let response = client.request(ControlRequest::Refresh).await?;
            let ControlResponse::Devices(devices) = response else {
                return response_error(response);
            };
            print_devices(&devices);
        }
        Command::Shutdown => expect_ok(client.request(ControlRequest::Shutdown).await?)?,
        Command::Daemon { .. } => unreachable!("daemon handled before client dispatch"),
    }
    Ok(())
}

async fn stream_logs(client: &DaemonClient, follow: bool, json: bool, limit: usize) -> Result<()> {
    let mut after = 0_u64;
    loop {
        let response = client
            .request(ControlRequest::Logs { after, limit })
            .await
            .context(service_hint())?;
        let ControlResponse::Logs(records) = response else {
            return response_error(response);
        };
        for record in &records {
            print_log(record, json)?;
            after = after.max(record.id);
        }
        if !follow {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

fn expect_ok(response: ControlResponse) -> Result<()> {
    match response {
        ControlResponse::Ok => Ok(()),
        other => response_error(other),
    }
}

fn response_error<T>(response: ControlResponse) -> Result<T> {
    match response {
        ControlResponse::Error { message } => bail!(message),
        other => bail!("unexpected daemon response: {other:?}"),
    }
}

fn print_devices(devices: &[DeviceSummary]) {
    if devices.is_empty() {
        println!("No MTP devices connected.");
        return;
    }
    for device in devices {
        println!(
            "{} {}  serial={}  writable={}",
            device.manufacturer, device.model, device.serial, device.writable
        );
        for storage in &device.storages {
            println!(
                "  {}  {} free / {}  writable={}",
                storage.name,
                format_bytes(storage.free_bytes),
                format_bytes(storage.total_bytes),
                storage.writable
            );
        }
    }
}

fn print_log(record: &LogRecord, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(record)?);
    } else {
        println!(
            "{} {:?} {:<10} {}",
            record.unix_millis, record.level, record.target, record.message
        );
    }
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn service_hint() -> &'static str {
    "MTPDrive service is not running; start the app or run `mtpdrive daemon`"
}
