use crate::model::{ControlRequest, ControlResponse, ServiceSnapshot};
use crate::{AppPaths, Error, Result};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Small JSON-lines client for the per-user daemon socket.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn discover() -> Result<Self> {
        Ok(Self::new(AppPaths::discover()?.socket_path))
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

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
            return Err(Error::InvalidResponse("empty response".into()));
        }
        Ok(serde_json::from_str(&line)?)
    }

    pub async fn is_running(&self) -> bool {
        matches!(
            self.request(ControlRequest::Ping).await,
            Ok(ControlResponse::Pong { .. })
        )
    }

    pub async fn snapshot(&self) -> Result<ServiceSnapshot> {
        match self.request(ControlRequest::Snapshot).await? {
            ControlResponse::Snapshot(snapshot) => Ok(snapshot),
            ControlResponse::Error { message } => Err(Error::Operation(message)),
            other => Err(Error::InvalidResponse(format!(
                "expected snapshot, got {other:?}"
            ))),
        }
    }
}
