use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "mtpdrive",
    version,
    about = "Expose Android MTP devices as a native macOS NFS volume"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::doc_markdown)]
pub(crate) enum Command {
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

#[cfg(test)]
#[path = "../tests/unit/args.rs"]
mod tests;
