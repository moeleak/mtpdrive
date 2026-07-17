use crate::settings::AppSettings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A storage area exposed by an MTP device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSummary {
    pub id: u64,
    pub name: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub writable: bool,
}

/// Information shown for a connected MTP device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub key: String,
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    pub device_version: String,
    pub usb_speed: Option<String>,
    pub generation: u64,
    pub writable: bool,
    pub storages: Vec<StorageSummary>,
}

/// Current state of the NFS mount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MountState {
    Unmounted,
    Mounting,
    Mounted { path: PathBuf, port: u16 },
    Error { message: String },
}

impl Default for MountState {
    fn default() -> Self {
        Self::Unmounted
    }
}

/// Complete UI-facing daemon snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub version: String,
    pub mount: MountState,
    pub devices: Vec<DeviceSummary>,
    pub transfer_count: usize,
    pub last_error: Option<String>,
}

impl Default for ServiceSnapshot {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            mount: MountState::Unmounted,
            devices: Vec::new(),
            transfer_count: 0,
            last_error: None,
        }
    }
}

/// Log severity shared between the daemon and Material log viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// One structured log record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    pub id: u64,
    pub unix_millis: i128,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
}

/// Versioned JSON-lines requests accepted by the daemon control socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlRequest {
    Ping,
    Snapshot,
    Devices,
    Logs { after: u64, limit: usize },
    Settings,
    SetSettings { settings: AppSettings },
    Mount,
    Unmount,
    Open,
    Refresh,
    Shutdown,
}

/// Versioned JSON-lines responses returned by the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum ControlResponse {
    Ok,
    Pong { version: String },
    Snapshot(ServiceSnapshot),
    Devices(Vec<DeviceSummary>),
    Logs(Vec<LogRecord>),
    Settings(AppSettings),
    Error { message: String },
}

#[cfg(test)]
#[path = "../tests/unit/model.rs"]
mod tests;
