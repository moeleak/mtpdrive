use crate::device::DeviceManager;
use crate::i18n::{current_language, set_current_language};
use crate::ipc::DaemonClient;
use crate::logs::LogStore;
use crate::model::{ControlRequest, ControlResponse, LogLevel, ServiceSnapshot};
use crate::mount::MountManager;
use crate::nfs::{MtpNfsFileSystem, NFS_FSID};
use crate::staging::StagingArea;
use crate::{AppPaths, AppSettings, Error, Result};
use fractal_nfs::NfsServerConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixListener, UnixStream};
use tokio::sync::{RwLock, watch};
use tokio_util::sync::CancellationToken;

/// Daemon startup behavior.
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub no_mount: bool,
    pub port: u16,
    pub open_on_first_device: bool,
    pub scan_interval: Duration,
}

impl Default for DaemonOptions {
    fn default() -> Self {
        Self {
            no_mount: false,
            port: 0,
            open_on_first_device: true,
            scan_interval: Duration::from_secs(2),
        }
    }
}

/// Long-running process that owns USB sessions, NFS, mounting, logs, and IPC.
pub struct MtpDriveDaemon {
    paths: AppPaths,
    options: DaemonOptions,
    logs: LogStore,
    manager: DeviceManager,
    filesystem: MtpNfsFileSystem,
    mount: MountManager,
    snapshot: Arc<RwLock<ServiceSnapshot>>,
    settings: Arc<RwLock<AppSettings>>,
}

impl std::fmt::Debug for MtpDriveDaemon {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MtpDriveDaemon")
            .field("paths", &self.paths)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl MtpDriveDaemon {
    pub fn new(paths: AppPaths, options: DaemonOptions) -> Result<Self> {
        paths.ensure()?;
        let settings = AppSettings::load(&paths)?;
        set_current_language(settings.language.resolve());
        let logs = LogStore::new(&paths.log_dir)?;
        let manager = DeviceManager::new(logs.clone());
        let staging = StagingArea::new(&paths.staging_dir, logs.clone())?;
        let filesystem =
            MtpNfsFileSystem::new(manager.clone(), staging, &paths.sidecar_dir, logs.clone())?;
        let mount = MountManager::new(paths.clone(), logs.clone());
        Ok(Self {
            paths,
            options,
            logs,
            manager,
            filesystem,
            mount,
            snapshot: Arc::new(RwLock::new(ServiceSnapshot::default())),
            settings: Arc::new(RwLock::new(settings)),
        })
    }

    pub async fn run(self) -> Result<()> {
        let language = current_language();
        self.ensure_single_instance().await?;
        let ipc_listener = UnixListener::bind(&self.paths.socket_path)?;
        let nfs_port = available_port(self.options.port)?;
        self.logs.emit(
            LogLevel::Info,
            "daemon",
            language.daemon_starting(env!("CARGO_PKG_VERSION")),
        );
        self.logs
            .emit(LogLevel::Info, "nfs", language.nfs_listening(nfs_port));

        let nfs_shutdown = CancellationToken::new();
        let server_shutdown = nfs_shutdown.clone();
        let filesystem = self.filesystem.clone();
        let nfs_task = tokio::task::spawn_blocking(move || {
            fractal_nfs::run_until(
                filesystem,
                NfsServerConfig {
                    port: nfs_port,
                    num_threads: 1,
                    fsid: NFS_FSID,
                    max_rpc_fragment_bytes: 16 * 1024 * 1024,
                },
                server_shutdown,
            )
        });
        if let Err(error) = wait_for_nfs(nfs_port).await {
            nfs_shutdown.cancel();
            let server_error = nfs_task
                .await
                .map_err(|join_error| Error::Operation(join_error.to_string()))?
                .err()
                .map_or_else(|| error.to_string(), |error| error.to_string());
            let _ = std::fs::remove_file(&self.paths.socket_path);
            return Err(Error::Operation(language.nfs_start_failed(server_error)));
        }
        if !self.options.no_mount
            && let Err(error) = self.mount.mount(nfs_port).await
        {
            self.logs.emit(LogLevel::Error, "mount", error.to_string());
        }
        self.update_snapshot().await;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let discovery_task = tokio::spawn(discovery_loop(
            self.manager.clone(),
            self.filesystem.clone(),
            self.mount.clone(),
            self.snapshot.clone(),
            self.logs.clone(),
            self.options.scan_interval,
            self.options.open_on_first_device && !self.options.no_mount,
            self.settings.clone(),
            shutdown_rx.clone(),
        ));

        let shared = Arc::new(DaemonShared {
            manager: self.manager.clone(),
            mount: self.mount.clone(),
            logs: self.logs.clone(),
            snapshot: self.snapshot.clone(),
            nfs_port,
            shutdown: shutdown_tx.clone(),
            settings: self.settings.clone(),
            paths: self.paths.clone(),
        });

        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                accepted = ipc_listener.accept() => {
                    let (stream, _) = accepted?;
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handle_client(stream, shared).await {
                            tracing::debug!(%error, "control connection failed");
                        }
                    });
                }
                result = tokio::signal::ctrl_c() => {
                    result?;
                    let _ = shutdown_tx.send(true);
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        self.logs
            .emit(LogLevel::Info, "daemon", language.daemon_shutting_down());
        discovery_task.abort();
        if let Err(error) = self.filesystem.flush_dirty().await {
            self.logs.emit(
                LogLevel::Warn,
                "transfer",
                language.flush_before_exit_failed(error),
            );
        }
        if !self.options.no_mount {
            if let Err(error) = self.mount.unmount().await {
                self.logs.emit(LogLevel::Warn, "mount", error.to_string());
            }
        }
        nfs_shutdown.cancel();
        match tokio::time::timeout(Duration::from_secs(3), nfs_task).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                self.logs.emit(
                    LogLevel::Warn,
                    "nfs",
                    language.nfs_stopped_with_error(error),
                );
            }
            Ok(Err(error)) => {
                self.logs
                    .emit(LogLevel::Warn, "nfs", language.nfs_task_failed(error));
            }
            Err(_) => {
                self.logs
                    .emit(LogLevel::Warn, "nfs", language.nfs_shutdown_timeout());
            }
        }
        let _ = std::fs::remove_file(&self.paths.socket_path);
        Ok(())
    }

    async fn ensure_single_instance(&self) -> Result<()> {
        if self.paths.socket_path.exists() {
            let client = DaemonClient::new(&self.paths.socket_path);
            if client.is_running().await {
                return Err(Error::Operation(
                    current_language().daemon_already_running().into(),
                ));
            }
            std::fs::remove_file(&self.paths.socket_path)?;
        }
        Ok(())
    }

    async fn update_snapshot(&self) {
        let devices = self.manager.summaries().await.unwrap_or_default();
        let mount = self.mount.state().await;
        let last_error = self.manager.last_error().await;
        *self.snapshot.write().await = ServiceSnapshot {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            mount,
            devices,
            transfer_count: 0,
            last_error,
        };
    }
}

fn available_port(requested: u16) -> Result<u16> {
    if requested != 0 {
        return Ok(requested);
    }
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

async fn wait_for_nfs(port: u16) -> Result<()> {
    let address = (std::net::Ipv4Addr::LOCALHOST, port);
    for _ in 0..100 {
        if TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(Error::Operation(
        current_language().wait_for_nfs_timeout(port),
    ))
}

struct DaemonShared {
    manager: DeviceManager,
    mount: MountManager,
    logs: LogStore,
    snapshot: Arc<RwLock<ServiceSnapshot>>,
    nfs_port: u16,
    shutdown: watch::Sender<bool>,
    settings: Arc<RwLock<AppSettings>>,
    paths: AppPaths,
}

async fn discovery_loop(
    manager: DeviceManager,
    filesystem: MtpNfsFileSystem,
    mount: MountManager,
    snapshot: Arc<RwLock<ServiceSnapshot>>,
    logs: LogStore,
    interval: Duration,
    allow_open_on_first_device: bool,
    settings: Arc<RwLock<AppSettings>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut previous_count = 0_usize;
    let mut storage_tick = 0_u32;
    let mut first_scan = true;
    loop {
        if first_scan {
            first_scan = false;
        } else {
            tokio::select! {
                () = tokio::time::sleep(interval) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                    continue;
                }
            }
        }

        let devices = match manager.refresh().await {
            Ok(devices) => devices,
            Err(error) => {
                logs.emit(
                    LogLevel::Warn,
                    "mtp",
                    current_language().device_scan_failed(error),
                );
                Vec::new()
            }
        };
        storage_tick = storage_tick.wrapping_add(1);
        if !devices.is_empty()
            && storage_tick.is_multiple_of(5)
            && let Err(error) = filesystem.flush_dirty().await
        {
            logs.emit(
                LogLevel::Warn,
                "transfer",
                current_language().reconnect_commit_failed(error),
            );
        }
        if storage_tick.is_multiple_of(15) {
            manager.refresh_storage_info().await;
        }
        if allow_open_on_first_device
            && settings.read().await.always_open_in_finder
            && previous_count == 0
            && !devices.is_empty()
        {
            let _ = mount.open_in_finder().await;
        }
        previous_count = devices.len();
        let mount_state = mount.state().await;
        let mut current = snapshot.write().await;
        current.devices = devices;
        current.mount = mount_state;
        current.last_error = manager.last_error().await;
    }
}

async fn handle_client(stream: UnixStream, shared: Arc<DaemonShared>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }
    let request: ControlRequest = serde_json::from_str(&line)?;
    let response = match request {
        ControlRequest::Ping => ControlResponse::Pong {
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        ControlRequest::Snapshot => {
            let mut snapshot = shared.snapshot.read().await.clone();
            snapshot.mount = shared.mount.state().await;
            snapshot.devices = shared.manager.summaries().await.unwrap_or_default();
            ControlResponse::Snapshot(snapshot)
        }
        ControlRequest::Devices => {
            ControlResponse::Devices(shared.manager.summaries().await.unwrap_or_default())
        }
        ControlRequest::Logs { after, limit } => {
            ControlResponse::Logs(shared.logs.after(after, limit))
        }
        ControlRequest::Settings => ControlResponse::Settings(*shared.settings.read().await),
        ControlRequest::SetSettings { settings } => match settings.save(&shared.paths) {
            Ok(()) => {
                set_current_language(settings.language.resolve());
                *shared.settings.write().await = settings;
                ControlResponse::Settings(settings)
            }
            Err(error) => ControlResponse::Error {
                message: error.to_string(),
            },
        },
        ControlRequest::Mount => match shared.mount.mount(shared.nfs_port).await {
            Ok(()) => ControlResponse::Ok,
            Err(error) => ControlResponse::Error {
                message: error.to_string(),
            },
        },
        ControlRequest::Unmount => match shared.mount.unmount().await {
            Ok(()) => ControlResponse::Ok,
            Err(error) => ControlResponse::Error {
                message: error.to_string(),
            },
        },
        ControlRequest::Open => match shared.mount.open_in_finder().await {
            Ok(()) => ControlResponse::Ok,
            Err(error) => ControlResponse::Error {
                message: error.to_string(),
            },
        },
        ControlRequest::Refresh => {
            shared.manager.invalidate_caches().await;
            match shared.manager.refresh().await {
                Ok(devices) => ControlResponse::Devices(devices),
                Err(error) => ControlResponse::Error {
                    message: error.to_string(),
                },
            }
        }
        ControlRequest::Shutdown => {
            let _ = shared.shutdown.send(true);
            ControlResponse::Ok
        }
    };
    let mut bytes = serde_json::to_vec(&response)?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;
    write_half.shutdown().await?;
    Ok(())
}
