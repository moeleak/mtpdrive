use crate::model::{ControlRequest, ControlResponse, DeviceSummary, LogRecord, ServiceSnapshot};
use crate::{AppPaths, AppSettings, Error, Result, current_language};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Small JSON-lines client for the per-user daemon socket.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
}

/// Errors returned by typed daemon requests.
#[derive(Debug)]
pub enum DaemonRequestError {
    Client(Error),
    Daemon(String),
    Unexpected(ControlResponse),
}

impl std::fmt::Display for DaemonRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(error) => error.fmt(formatter),
            Self::Daemon(message) => formatter.write_str(message),
            Self::Unexpected(response) => {
                formatter.write_str(&current_language().unexpected_daemon_response(response))
            }
        }
    }
}

impl std::error::Error for DaemonRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            Self::Daemon(_) | Self::Unexpected(_) => None,
        }
    }
}

impl From<Error> for DaemonRequestError {
    fn from(error: Error) -> Self {
        Self::Client(error)
    }
}

/// Result returned by typed daemon requests.
pub type DaemonRequestResult<T> = std::result::Result<T, DaemonRequestError>;

impl DaemonClient {
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Discovers the current user's daemon socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the application paths cannot be discovered.
    pub fn discover() -> Result<Self> {
        Ok(Self::new(AppPaths::discover()?.socket_path))
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Sends a low-level control request to the daemon.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot be reached or the request or
    /// response cannot be serialized.
    pub async fn request(&self, request: ControlRequest) -> Result<ControlResponse> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) {
                    Error::DaemonUnavailable
                } else {
                    Error::Io(error)
                }
            })?;
        let (read_half, mut write_half) = stream.into_split();
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        write_half.write_all(&bytes).await?;
        write_half.shutdown().await?;

        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Err(Error::InvalidResponse(
                current_language().strings().empty_response.into(),
            ));
        }
        Ok(serde_json::from_str(&line)?)
    }

    pub async fn is_running(&self) -> bool {
        matches!(
            self.request(ControlRequest::Ping).await,
            Ok(ControlResponse::Pong { .. })
        )
    }

    /// Waits until the daemon responds or the timeout expires.
    pub async fn wait_until_running(&self, timeout: Duration, poll_interval: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.is_running().await {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Returns the daemon's current service snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn snapshot(&self) -> Result<ServiceSnapshot> {
        match self.request(ControlRequest::Snapshot).await? {
            ControlResponse::Snapshot(snapshot) => Ok(snapshot),
            ControlResponse::Error { message } => Err(Error::Operation(message)),
            other => Err(Error::InvalidResponse(
                current_language().expected_snapshot(other),
            )),
        }
    }

    /// Returns the currently connected devices.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn devices(&self) -> DaemonRequestResult<Vec<DeviceSummary>> {
        match self.request(ControlRequest::Devices).await? {
            ControlResponse::Devices(devices) => Ok(devices),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }

    /// Returns log records newer than `after`.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn logs(&self, after: u64, limit: usize) -> DaemonRequestResult<Vec<LogRecord>> {
        match self.request(ControlRequest::Logs { after, limit }).await? {
            ControlResponse::Logs(records) => Ok(records),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }

    /// Returns the daemon settings.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn settings(&self) -> DaemonRequestResult<AppSettings> {
        match self.request(ControlRequest::Settings).await? {
            ControlResponse::Settings(settings) => Ok(settings),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }

    /// Saves daemon settings and returns the persisted value.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn set_settings(&self, settings: AppSettings) -> DaemonRequestResult<AppSettings> {
        match self
            .request(ControlRequest::SetSettings { settings })
            .await?
        {
            ControlResponse::Settings(settings) => Ok(settings),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }

    /// Mounts the network volume.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn mount(&self) -> DaemonRequestResult<()> {
        self.expect_ok(ControlRequest::Mount).await
    }

    /// Unmounts the network volume.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn unmount(&self) -> DaemonRequestResult<()> {
        self.expect_ok(ControlRequest::Unmount).await
    }

    /// Opens the network volume in Finder.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn open_in_finder(&self) -> DaemonRequestResult<()> {
        self.expect_ok(ControlRequest::Open).await
    }

    /// Rescans USB devices and returns the refreshed list.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn refresh(&self) -> DaemonRequestResult<Vec<DeviceSummary>> {
        match self.request(ControlRequest::Refresh).await? {
            ControlResponse::Devices(devices) => Ok(devices),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }

    /// Requests a clean daemon shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, daemon failures, or an
    /// unexpected response type.
    pub async fn shutdown(&self) -> DaemonRequestResult<()> {
        self.expect_ok(ControlRequest::Shutdown).await
    }

    async fn expect_ok(&self, request: ControlRequest) -> DaemonRequestResult<()> {
        match self.request(request).await? {
            ControlResponse::Ok => Ok(()),
            ControlResponse::Error { message } => Err(DaemonRequestError::Daemon(message)),
            other => Err(DaemonRequestError::Unexpected(other)),
        }
    }
}
