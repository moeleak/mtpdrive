//! Core services for `MTPDrive`.

pub mod daemon;
pub mod device;
pub mod format;
pub mod i18n;
pub mod ipc;
pub mod logs;
pub mod model;
pub mod mount;
pub mod nfs;
pub mod paths;
pub mod settings;
pub mod staging;

pub use daemon::{DaemonOptions, MtpDriveDaemon};
pub use format::{format_bytes, format_mount_state};
pub use i18n::{
    Language, Strings, current_language, detect_language_from, parse_language,
    set_current_language, system_language,
};
pub use ipc::{DaemonClient, DaemonRequestError, DaemonRequestResult};
pub use model::{
    ControlRequest, ControlResponse, DeviceSummary, LogLevel, LogRecord, MountState,
    ServiceSnapshot, StorageSummary,
};
pub use paths::AppPaths;
pub use settings::{AppSettings, LanguagePreference};

/// `MTPDrive`'s result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by core services.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Mtp(mtp_rs::Error),
    Json(serde_json::Error),
    DaemonUnavailable,
    InvalidResponse(String),
    Disconnected,
    NotFound,
    Unsupported(String),
    Operation(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let strings = current_language().strings();
        match self {
            Self::Io(error) => write!(formatter, "{}: {error}", strings.io_error),
            Self::Mtp(error) => write!(formatter, "{}: {error}", strings.mtp_error),
            Self::Json(error) => write!(formatter, "{}: {error}", strings.serialization_error),
            Self::DaemonUnavailable => formatter.write_str(strings.daemon_unavailable),
            Self::InvalidResponse(detail) => {
                write!(formatter, "{}: {detail}", strings.invalid_response)
            }
            Self::Disconnected => formatter.write_str(strings.disconnected),
            Self::NotFound => formatter.write_str(strings.not_found),
            Self::Unsupported(detail) => write!(formatter, "{}: {detail}", strings.unsupported),
            Self::Operation(detail) => {
                write!(formatter, "{}: {detail}", strings.operation_failed)
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Mtp(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::DaemonUnavailable
            | Self::InvalidResponse(_)
            | Self::Disconnected
            | Self::NotFound
            | Self::Unsupported(_)
            | Self::Operation(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<mtp_rs::Error> for Error {
    fn from(error: mtp_rs::Error) -> Self {
        Self::Mtp(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
