use crate::logs::LogStore;
use crate::model::{LogLevel, MountState};
use crate::{AppPaths, Error, Result};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;

/// Controls the per-user macOS NFS mount.
#[derive(Debug, Clone)]
pub struct MountManager {
    paths: AppPaths,
    state: Arc<RwLock<MountState>>,
    logs: LogStore,
}

impl MountManager {
    #[must_use]
    pub fn new(paths: AppPaths, logs: LogStore) -> Self {
        Self {
            paths,
            state: Arc::new(RwLock::new(MountState::Unmounted)),
            logs,
        }
    }

    pub async fn state(&self) -> MountState {
        self.state.read().await.clone()
    }

    pub async fn mount(&self, port: u16) -> Result<()> {
        if self.is_mounted().await {
            *self.state.write().await = MountState::Mounted {
                path: self.paths.mount_point.clone(),
                port,
            };
            return Ok(());
        }
        *self.state.write().await = MountState::Mounting;
        self.paths.ensure()?;

        #[cfg(target_os = "macos")]
        let result = Command::new("/sbin/mount_nfs")
            .arg("-o")
            .arg(format!(
                "vers=3,tcp,nolocks,localhost,port={port},mountport={port},rsize=131072,wsize=131072,actimeo=1,nodev,nosuid,soft,timeo=600,retrans=5"
            ))
            .arg("MTPDrive:/MTPDrive")
            .arg(&self.paths.mount_point)
            .output()
            .await?;

        #[cfg(not(target_os = "macos"))]
        let result = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: b"NFS mounting is only supported on macOS".to_vec(),
        };

        if result.status.success() {
            *self.state.write().await = MountState::Mounted {
                path: self.paths.mount_point.clone(),
                port,
            };
            self.logs.emit(
                LogLevel::Info,
                "mount",
                format!("mounted {}", self.paths.mount_point.display()),
            );
            Ok(())
        } else {
            let message = String::from_utf8_lossy(&result.stderr).trim().to_owned();
            *self.state.write().await = MountState::Error {
                message: message.clone(),
            };
            self.logs.emit(LogLevel::Error, "mount", &message);
            Err(Error::Operation(format!("mount_nfs failed: {message}")))
        }
    }

    pub async fn unmount(&self) -> Result<()> {
        if !self.is_mounted().await {
            *self.state.write().await = MountState::Unmounted;
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        let mut result = Command::new("/usr/sbin/diskutil")
            .arg("unmount")
            .arg(&self.paths.mount_point)
            .output()
            .await?;

        #[cfg(target_os = "macos")]
        if !result.status.success() {
            result = Command::new("/sbin/umount")
                .arg(&self.paths.mount_point)
                .output()
                .await?;
        }

        #[cfg(target_os = "macos")]
        if !result.status.success() {
            result = Command::new("/sbin/umount")
                .arg("-f")
                .arg(&self.paths.mount_point)
                .output()
                .await?;
        }

        #[cfg(not(target_os = "macos"))]
        let result = std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };

        if result.status.success() {
            *self.state.write().await = MountState::Unmounted;
            self.logs
                .emit(LogLevel::Info, "mount", "unmounted MTPDrive");
            Ok(())
        } else {
            let message = String::from_utf8_lossy(&result.stderr).trim().to_owned();
            *self.state.write().await = MountState::Error {
                message: message.clone(),
            };
            Err(Error::Operation(format!("unmount failed: {message}")))
        }
    }

    pub async fn open_in_finder(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let status = Command::new("/usr/bin/open")
                .arg(&self.paths.mount_point)
                .status()
                .await?;
            if !status.success() {
                return Err(Error::Operation("could not open Finder".into()));
            }
        }
        Ok(())
    }

    async fn is_mounted(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            let Ok(output) = Command::new("/sbin/mount").output().await else {
                return false;
            };
            let needle = format!(" on {} ", self.paths.mount_point.display());
            String::from_utf8_lossy(&output.stdout).contains(&needle)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}
