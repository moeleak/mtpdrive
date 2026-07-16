//! Core services for MTPDrive.

pub mod daemon;
pub mod device;
pub mod ipc;
pub mod logs;
pub mod model;
pub mod mount;
pub mod nfs;
pub mod paths;
pub mod staging;

pub use daemon::{DaemonOptions, MtpDriveDaemon};
pub use ipc::DaemonClient;
pub use model::{
    ControlRequest, ControlResponse, DeviceSummary, LogLevel, LogRecord, MountState,
    ServiceSnapshot, StorageSummary,
};
pub use paths::AppPaths;

/// MTPDrive's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by core services.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("MTP error: {0}")]
    Mtp(#[from] mtp_rs::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("daemon is not running")]
    DaemonUnavailable,
    #[error("invalid response from daemon: {0}")]
    InvalidResponse(String),
    #[error("device is disconnected")]
    Disconnected,
    #[error("object was not found")]
    NotFound,
    #[error("operation is not supported: {0}")]
    Unsupported(String),
    #[error("operation failed: {0}")]
    Operation(String),
}
