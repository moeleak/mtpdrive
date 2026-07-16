use crate::logs::LogStore;
use crate::model::{DeviceSummary, LogLevel, StorageSummary};
use crate::{Error, Result};
use bytes::Bytes;
use futures::stream;
use mtp_rs::mtp::{
    MtpDevice, MtpDeviceInfo, NewObjectInfo, ObjectHandle, ObjectInfo, Storage, StorageId,
};
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::io::ReaderStream;

#[derive(Debug, Clone)]
pub struct ObjectEntry {
    pub handle: u64,
    pub storage_id: u64,
    pub parent: u64,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub created: Option<mtp_rs::mtp::DateTime>,
    pub modified: Option<mtp_rs::mtp::DateTime>,
}

impl From<ObjectInfo> for ObjectEntry {
    fn from(info: ObjectInfo) -> Self {
        let is_dir = info.is_folder();
        Self {
            handle: info.handle.0,
            storage_id: info.storage_id.0,
            parent: normalize_parent(info.parent.0),
            name: info.filename,
            size: info.size,
            is_dir,
            created: info.created,
            modified: info.modified,
        }
    }
}

struct ConnectedDevice {
    key: String,
    physical_key: String,
    vendor_product_key: String,
    device: MtpDevice,
    gate: Semaphore,
    summary: RwLock<DeviceSummary>,
}

impl ConnectedDevice {
    async fn refresh_storages(&self) -> Result<()> {
        let _permit = self.gate.acquire().await.map_err(|_| Error::Disconnected)?;
        let storages = self.device.storages().await?;
        let mut summary = self.summary.write().await;
        summary.storages = storages
            .iter()
            .map(|storage| StorageSummary {
                id: storage.id().0,
                name: storage_name(storage.info().description.as_str(), storage.id().0),
                total_bytes: storage.info().total_capacity,
                free_bytes: storage.info().free_space,
                writable: storage.info().is_writable,
            })
            .collect();
        summary.writable = summary.storages.iter().any(|storage| storage.writable)
            && self.device.supports_upload();
        Ok(())
    }
}

/// Owns all open MTP sessions and serializes operations per physical device.
#[derive(Clone)]
pub struct DeviceManager {
    devices: Arc<RwLock<HashMap<String, Arc<ConnectedDevice>>>>,
    open_failures: Arc<RwLock<HashMap<String, String>>>,
    last_error: Arc<RwLock<Option<String>>>,
    last_change_millis: Arc<AtomicU64>,
    next_generation: Arc<AtomicU64>,
    logs: LogStore,
}

impl std::fmt::Debug for DeviceManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceManager")
            .field(
                "device_count",
                &self.devices.try_read().map(|map| map.len()),
            )
            .finish_non_exhaustive()
    }
}

impl DeviceManager {
    #[must_use]
    pub fn new(logs: LogStore) -> Self {
        Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            open_failures: Arc::new(RwLock::new(HashMap::new())),
            last_error: Arc::new(RwLock::new(None)),
            last_change_millis: Arc::new(AtomicU64::new(now_millis())),
            next_generation: Arc::new(AtomicU64::new(1)),
            logs,
        }
    }

    /// Enumerates USB devices, opens new sessions, and removes disconnected sessions.
    pub async fn refresh(&self) -> Result<Vec<DeviceSummary>> {
        let candidates = tokio::task::spawn_blocking(MtpDevice::list_devices)
            .await
            .map_err(|error| {
                Error::Operation(format!("device enumeration task failed: {error}"))
            })??;
        let mut unique = HashMap::<String, MtpDeviceInfo>::new();
        for candidate in candidates {
            let physical_key = physical_device_key(&candidate);
            match unique.entry(physical_key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(candidate);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry.get().serial_number.is_none() && candidate.serial_number.is_some() {
                        entry.insert(candidate);
                    }
                }
            }
        }
        let candidates: Vec<MtpDeviceInfo> = unique.into_values().collect();
        let candidate_keys: HashSet<String> = candidates.iter().map(device_key).collect();
        self.open_failures
            .write()
            .await
            .retain(|key, _| candidate_keys.contains(key));

        let connected: Vec<Arc<ConnectedDevice>> =
            self.devices.read().await.values().cloned().collect();
        for device in connected {
            let present = candidates
                .iter()
                .any(|candidate| candidate_matches_connected(candidate, &device));
            if !present && device.refresh_storages().await.is_err() {
                self.devices.write().await.remove(&device.key);
                self.mark_changed();
                self.logs.emit(
                    LogLevel::Info,
                    "mtp",
                    format!("device disconnected: {}", device.key),
                );
            }
        }

        let mut latest_error = None;
        for candidate in candidates {
            let key = device_key(&candidate);
            if self
                .devices
                .read()
                .await
                .values()
                .any(|device| candidate_matches_connected(&candidate, device))
            {
                continue;
            }
            match self.open_candidate(&candidate, key.clone()).await {
                Ok(connected) => {
                    self.open_failures.write().await.remove(&key);
                    self.logs.emit(
                        LogLevel::Info,
                        "mtp",
                        format!(
                            "connected to {} {}",
                            connected.summary.read().await.manufacturer,
                            connected.summary.read().await.model
                        ),
                    );
                    self.devices.write().await.insert(key, Arc::new(connected));
                    self.mark_changed();
                }
                Err(error) => {
                    let message = open_error_message(&key, &error);
                    let changed = self
                        .open_failures
                        .read()
                        .await
                        .get(&key)
                        .is_none_or(|previous| previous != &message);
                    if changed {
                        self.logs.emit(LogLevel::Warn, "mtp", &message);
                    }
                    self.open_failures
                        .write()
                        .await
                        .insert(key, message.clone());
                    latest_error = Some(message);
                }
            }
        }

        *self.last_error.write().await = latest_error;

        self.summaries().await
    }

    pub async fn refresh_storage_info(&self) {
        let devices: Vec<Arc<ConnectedDevice>> =
            self.devices.read().await.values().cloned().collect();
        for device in devices {
            if let Err(error) = device.refresh_storages().await {
                self.logs.emit(
                    LogLevel::Warn,
                    "mtp",
                    format!("failed to refresh storage for {}: {error}", device.key),
                );
            } else {
                self.mark_changed();
            }
        }
    }

    pub async fn summaries(&self) -> Result<Vec<DeviceSummary>> {
        let devices: Vec<Arc<ConnectedDevice>> =
            self.devices.read().await.values().cloned().collect();
        let mut summaries = Vec::with_capacity(devices.len());
        for device in devices {
            summaries.push(device.summary.read().await.clone());
        }
        summaries.sort_by(|left, right| {
            left.model
                .cmp(&right.model)
                .then_with(|| left.serial.cmp(&right.serial))
        });
        Ok(summaries)
    }

    pub async fn summary(&self, key: &str) -> Result<DeviceSummary> {
        let device = self.get(key).await?;
        let summary = device.summary.read().await.clone();
        Ok(summary)
    }

    pub async fn last_error(&self) -> Option<String> {
        self.last_error.read().await.clone()
    }

    #[must_use]
    pub fn last_change(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.last_change_millis.load(Ordering::Acquire))
    }

    fn mark_changed(&self) {
        let now = now_millis();
        self.last_change_millis.fetch_max(now, Ordering::Release);
    }

    pub async fn list(
        &self,
        device_key: &str,
        storage_id: u64,
        parent: Option<u64>,
    ) -> Result<Vec<ObjectEntry>> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        let parent = parent.map(ObjectHandle);
        Ok(storage
            .list_objects(parent)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn metadata(
        &self,
        device_key: &str,
        storage_id: u64,
        handle: u64,
    ) -> Result<ObjectEntry> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        Ok(storage.get_object_info(ObjectHandle(handle)).await?.into())
    }

    pub async fn read(
        &self,
        device_key: &str,
        storage_id: u64,
        handle: u64,
        offset: u64,
        count: u32,
    ) -> Result<Bytes> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        Ok(Bytes::from(
            storage
                .read_range(ObjectHandle(handle), offset, count)
                .await?,
        ))
    }

    pub async fn download_to(
        &self,
        device_key: &str,
        storage_id: u64,
        handle: u64,
        path: &Path,
    ) -> Result<()> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        let mut download = storage
            .download_windowed_default(ObjectHandle(handle))
            .await?;
        let mut file = tokio::fs::File::create(path).await?;
        use tokio::io::AsyncWriteExt;
        while let Some(bytes) = download.next_window().await {
            file.write_all(&bytes?).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }

    pub async fn create_folder(
        &self,
        device_key: &str,
        storage_id: u64,
        parent: Option<u64>,
        name: &str,
    ) -> Result<u64> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        let listing_parent = parent.map(ObjectHandle);
        let handle = storage
            .create_folder(mtp_write_parent(parent), name)
            .await?;
        Ok(storage
            .list_objects(listing_parent)
            .await
            .ok()
            .and_then(|objects| find_uploaded_object(&objects, name, 0, Some(handle), Some(true)))
            .unwrap_or(handle)
            .0)
    }

    pub async fn delete(&self, device_key: &str, storage_id: u64, handle: u64) -> Result<()> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        storage.delete(ObjectHandle(handle)).await?;
        Ok(())
    }

    pub async fn rename(
        &self,
        device_key: &str,
        storage_id: u64,
        handle: u64,
        new_name: &str,
    ) -> Result<()> {
        let device = self.get(device_key).await?;
        if !device.device.capabilities().can_rename {
            return Err(Error::Unsupported(
                "device does not advertise rename".into(),
            ));
        }
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        storage.rename(ObjectHandle(handle), new_name).await?;
        Ok(())
    }

    pub async fn move_object(
        &self,
        device_key: &str,
        source_storage_id: u64,
        handle: u64,
        destination_storage_id: u64,
        destination_parent: Option<u64>,
    ) -> Result<()> {
        let device = self.get(device_key).await?;
        if !device.device.capabilities().can_move {
            return Err(Error::Unsupported("device does not advertise move".into()));
        }
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(source_storage_id)).await?;
        storage
            .move_object(
                ObjectHandle(handle),
                destination_parent.map_or(ObjectHandle::ALL, ObjectHandle),
                Some(StorageId(destination_storage_id)),
            )
            .await?;
        Ok(())
    }

    pub async fn upload_file(
        &self,
        device_key: &str,
        storage_id: u64,
        parent: Option<u64>,
        name: &str,
        path: &Path,
    ) -> Result<u64> {
        let device = self.get(device_key).await?;
        if !device.device.capabilities().can_upload {
            return Err(Error::Unsupported(
                "device does not advertise upload".into(),
            ));
        }
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        let stream = ReaderStream::new(file);
        let listing_parent = parent.map(ObjectHandle);
        let result = storage
            .upload_with_progress(
                mtp_write_parent(parent),
                NewObjectInfo::file(name, size),
                stream,
                |_progress| ControlFlow::Continue(()),
            )
            .await;
        match result {
            Ok(handle) => {
                Ok(
                    canonical_uploaded_handle(&storage, listing_parent, name, size, handle)
                        .await
                        .0,
                )
            }
            Err(upload_error) => {
                if upload_error.source.is_stale_handle()
                    && let Some(handle) = find_uploaded_handle(
                        &storage,
                        listing_parent,
                        name,
                        size,
                        upload_error.partial,
                    )
                    .await
                {
                    return Ok(handle.0);
                }
                if let Some(partial) = upload_error.partial {
                    let _ = storage.delete(partial).await;
                }
                Err(Error::Mtp(upload_error.source))
            }
        }
    }

    /// Uploads bytes, used by small sidecar-oriented tests and utilities.
    pub async fn upload_bytes(
        &self,
        device_key: &str,
        storage_id: u64,
        parent: Option<u64>,
        name: &str,
        data: Bytes,
    ) -> Result<u64> {
        let device = self.get(device_key).await?;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.device.storage(StorageId(storage_id)).await?;
        let size = data.len() as u64;
        let body = stream::iter([Ok::<Bytes, std::io::Error>(data)]);
        let listing_parent = parent.map(ObjectHandle);
        let result = storage
            .upload(
                mtp_write_parent(parent),
                NewObjectInfo::file(name, size),
                body,
            )
            .await;
        match result {
            Ok(handle) => {
                Ok(
                    canonical_uploaded_handle(&storage, listing_parent, name, size, handle)
                        .await
                        .0,
                )
            }
            Err(upload_error) => {
                if upload_error.source.is_stale_handle()
                    && let Some(handle) = find_uploaded_handle(
                        &storage,
                        listing_parent,
                        name,
                        size,
                        upload_error.partial,
                    )
                    .await
                {
                    return Ok(handle.0);
                }
                if let Some(partial) = upload_error.partial {
                    let _ = storage.delete(partial).await;
                }
                Err(Error::Mtp(upload_error.source))
            }
        }
    }

    async fn open_candidate(
        &self,
        candidate: &MtpDeviceInfo,
        key: String,
    ) -> Result<ConnectedDevice> {
        let device = if let Some(serial) = candidate.serial_number.as_deref() {
            MtpDevice::open_by_serial(serial).await?
        } else {
            MtpDevice::open_by_location(candidate.location_id).await?
        };
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let storages = device.storages().await?;
        let info = device.device_info();
        let writable = storages.iter().any(|storage| storage.info().is_writable)
            && device.capabilities().can_upload;
        let summary = DeviceSummary {
            key: key.clone(),
            manufacturer: nonempty(
                &info.manufacturer,
                candidate.manufacturer.as_deref(),
                "Unknown",
            ),
            model: nonempty(&info.model, candidate.product.as_deref(), "MTP device"),
            serial: nonempty(
                &info.serial_number,
                candidate.serial_number.as_deref(),
                &format!("location-{:x}", candidate.location_id),
            ),
            device_version: info.device_version.clone(),
            usb_speed: candidate.speed.map(|speed| format!("{speed:?}")),
            generation,
            writable,
            storages: storages
                .iter()
                .map(|storage| StorageSummary {
                    id: storage.id().0,
                    name: storage_name(storage.info().description.as_str(), storage.id().0),
                    total_bytes: storage.info().total_capacity,
                    free_bytes: storage.info().free_space,
                    writable: storage.info().is_writable,
                })
                .collect(),
        };
        Ok(ConnectedDevice {
            key,
            physical_key: physical_device_key(candidate),
            vendor_product_key: vendor_product_key(candidate),
            device,
            gate: Semaphore::new(1),
            summary: RwLock::new(summary),
        })
    }

    async fn get(&self, key: &str) -> Result<Arc<ConnectedDevice>> {
        self.devices
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or(Error::Disconnected)
    }
}

fn device_key(info: &MtpDeviceInfo) -> String {
    info.serial_number.as_ref().map_or_else(
        || {
            format!(
                "{:04x}:{:04x}@{:x}",
                info.vendor_id, info.product_id, info.location_id
            )
        },
        |serial| format!("{:04x}:{:04x}:{serial}", info.vendor_id, info.product_id),
    )
}

fn physical_device_key(info: &MtpDeviceInfo) -> String {
    format!(
        "{:04x}:{:04x}@{:x}",
        info.vendor_id, info.product_id, info.location_id
    )
}

fn vendor_product_key(info: &MtpDeviceInfo) -> String {
    format!("{:04x}:{:04x}", info.vendor_id, info.product_id)
}

fn has_useful_serial(info: &MtpDeviceInfo) -> bool {
    info.serial_number
        .as_deref()
        .is_some_and(|serial| !serial.trim().is_empty() && serial != "?")
}

fn candidate_matches_connected(candidate: &MtpDeviceInfo, connected: &ConnectedDevice) -> bool {
    device_key(candidate) == connected.key
        || physical_device_key(candidate) == connected.physical_key
        || (!has_useful_serial(candidate)
            && vendor_product_key(candidate) == connected.vendor_product_key)
}

fn open_error_message(key: &str, error: &Error) -> String {
    let detail = error.to_string();
    if detail.contains("held exclusively by another process") {
        format!(
            "无法打开 Android 设备 {key}：它正被另一个程序独占。请关闭“预览”、“照片”或“图像捕捉”等正在访问手机的程序，重新连接 USB，并在手机上选择“文件传输 / Android Auto”。"
        )
    } else {
        format!("无法打开 Android 设备 {key}：{detail}")
    }
}

fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn nonempty(primary: &str, secondary: Option<&str>, fallback: &str) -> String {
    if !primary.trim().is_empty() {
        primary.to_owned()
    } else if let Some(value) = secondary.filter(|value| !value.trim().is_empty()) {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn storage_name(description: &str, id: u64) -> String {
    if description.trim().is_empty() {
        format!("Storage {id:08x}")
    } else {
        description.to_owned()
    }
}

fn normalize_parent(parent: u64) -> u64 {
    if parent == ObjectHandle::ROOT.0 || parent == ObjectHandle::ALL.0 {
        ObjectHandle::ROOT.0
    } else {
        parent
    }
}

/// Android uses 0xffff_ffff for a storage-root parent in mutating MTP
/// operations. Keep `None` for root listings, but pass the wire sentinel
/// explicitly for writes because `mtp-rs` 0.23 currently maps `None` to zero.
fn mtp_write_parent(parent: Option<u64>) -> Option<ObjectHandle> {
    Some(parent.map_or(ObjectHandle::ALL, ObjectHandle))
}

async fn canonical_uploaded_handle(
    storage: &Storage,
    parent: Option<ObjectHandle>,
    name: &str,
    size: u64,
    reported: ObjectHandle,
) -> ObjectHandle {
    find_uploaded_handle(storage, parent, name, size, Some(reported))
        .await
        .unwrap_or(reported)
}

async fn find_uploaded_handle(
    storage: &Storage,
    parent: Option<ObjectHandle>,
    name: &str,
    size: u64,
    reported: Option<ObjectHandle>,
) -> Option<ObjectHandle> {
    let objects = storage.list_objects(parent).await.ok()?;
    find_uploaded_object(&objects, name, size, reported, Some(false))
}

fn find_uploaded_object(
    objects: &[ObjectInfo],
    name: &str,
    size: u64,
    reported: Option<ObjectHandle>,
    is_dir: Option<bool>,
) -> Option<ObjectHandle> {
    let matches = |object: &ObjectInfo| {
        object.filename == name
            && object.size == size
            && is_dir.is_none_or(|expected| object.is_folder() == expected)
    };
    if let Some(reported) = reported
        && let Some(object) = objects
            .iter()
            .find(|object| object.handle == reported && matches(object))
    {
        return Some(object.handle);
    }
    objects
        .iter()
        .find(|object| matches(object))
        .map(|object| object.handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_uses_each_fallback() {
        assert_eq!(nonempty("Pixel", Some("USB"), "Unknown"), "Pixel");
        assert_eq!(nonempty("", Some("USB"), "Unknown"), "USB");
        assert_eq!(nonempty("", None, "Unknown"), "Unknown");
    }

    #[test]
    fn both_mtp_root_parent_encodings_are_normalized() {
        assert_eq!(normalize_parent(ObjectHandle::ROOT.0), 0);
        assert_eq!(normalize_parent(ObjectHandle::ALL.0), 0);
        assert_eq!(normalize_parent(42), 42);
        assert_eq!(mtp_write_parent(None), Some(ObjectHandle::ALL));
        assert_eq!(mtp_write_parent(Some(42)), Some(ObjectHandle(42)));
    }
}
