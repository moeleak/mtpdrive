use crate::{Error, Result, current_language};
use directories::BaseDirs;
use std::path::{Path, PathBuf};

/// All filesystem paths used by MTPDrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub support_dir: PathBuf,
    pub settings_path: PathBuf,
    pub cache_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub sidecar_dir: PathBuf,
    pub log_dir: PathBuf,
    pub socket_path: PathBuf,
    pub mount_point: PathBuf,
}

impl AppPaths {
    /// Resolves the standard per-user macOS locations.
    pub fn discover() -> Result<Self> {
        if let Some(root) = std::env::var_os("MTPDRIVE_HOME") {
            return Ok(Self::under(Path::new(&root)));
        }

        let base = BaseDirs::new().ok_or_else(|| {
            Error::Operation(
                current_language()
                    .strings()
                    .home_directory_unavailable
                    .into(),
            )
        })?;
        let home = base.home_dir();
        let support = home
            .join("Library")
            .join("Application Support")
            .join("MTPDrive");
        let cache = home.join("Library").join("Caches").join("MTPDrive");

        Ok(Self {
            settings_path: support.join("settings.json"),
            staging_dir: cache.join("staging"),
            sidecar_dir: support.join("sidecar"),
            log_dir: support.join("logs"),
            socket_path: support.join("control.sock"),
            mount_point: home.join("MTPDrive"),
            support_dir: support,
            cache_dir: cache,
        })
    }

    /// Creates isolated paths below `root`, primarily for tests.
    #[must_use]
    pub fn under(root: &Path) -> Self {
        let support = root.join("support");
        let cache = root.join("cache");
        Self {
            settings_path: support.join("settings.json"),
            staging_dir: cache.join("staging"),
            sidecar_dir: support.join("sidecar"),
            log_dir: support.join("logs"),
            socket_path: support.join("control.sock"),
            mount_point: root.join("MTPDrive"),
            support_dir: support,
            cache_dir: cache,
        }
    }

    /// Creates all parent directories required before starting the daemon.
    pub fn ensure(&self) -> Result<()> {
        for path in [
            &self.support_dir,
            &self.cache_dir,
            &self.staging_dir,
            &self.sidecar_dir,
            &self.log_dir,
            &self.mount_point,
        ] {
            std::fs::create_dir_all(path)?;
        }
        Ok(())
    }
}
