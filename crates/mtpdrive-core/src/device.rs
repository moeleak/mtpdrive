use crate::logs::LogStore;
use crate::model::{DeviceSummary, LogLevel, StorageSummary};
use crate::{Error, Result, current_language};
use bytes::Bytes;
use futures::stream;
use mtp_rs::CancelToken;
use mtp_rs::mtp::{
    DeviceEvent, Error as MtpError, MtpDevice, MtpDeviceInfo, NewObjectInfo, ObjectHandle,
    ObjectInfo, Storage, StorageId,
};
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio_util::io::ReaderStream;

const DIRECTORY_CACHE_TTL: Duration = Duration::from_secs(10);
const DIRECTORY_FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const EVENT_POLL_TIMEOUT: Duration = Duration::from_secs(1);
const IMAGE_CAPTURE_RECLAIM_COOLDOWN: Duration = Duration::from_secs(10);
const IMAGE_CAPTURE_DAEMON_PATH: &str = "/System/Library/Image Capture/Support/icdd";

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DirectoryCacheKey {
    storage_id: u64,
    parent: Option<u64>,
}

impl DirectoryCacheKey {
    fn new(storage_id: u64, parent: Option<u64>) -> Self {
        Self {
            storage_id,
            parent: parent.filter(|parent| *parent != ObjectHandle::ROOT.0),
        }
    }
}

#[derive(Debug, Clone)]
struct CachedListing {
    loaded_at: Instant,
    fully_loaded_at: Instant,
    entries: Vec<ObjectEntry>,
}

#[derive(Debug, Clone)]
struct DirectoryRefresh {
    epoch: u64,
    cancel: CancelToken,
}

#[derive(Debug, Default)]
struct DeviceCache {
    listings: HashMap<DirectoryCacheKey, CachedListing>,
    objects: HashMap<(u64, u64), ObjectEntry>,
    refreshing: HashMap<DirectoryCacheKey, DirectoryRefresh>,
    epoch: u64,
}

impl DeviceCache {
    fn listing(
        &self,
        storage_id: u64,
        parent: Option<u64>,
        now: Instant,
    ) -> Option<(Vec<ObjectEntry>, bool)> {
        self.listings
            .get(&DirectoryCacheKey::new(storage_id, parent))
            .map(|cached| {
                (
                    cached.entries.clone(),
                    is_cache_fresh(cached.loaded_at, now),
                )
            })
    }

    fn object(&self, storage_id: u64, handle: u64) -> Option<ObjectEntry> {
        self.objects.get(&(storage_id, handle)).cloned()
    }

    fn begin_refresh(
        &mut self,
        storage_id: u64,
        parent: Option<u64>,
    ) -> Option<(u64, CancelToken)> {
        let key = DirectoryCacheKey::new(storage_id, parent);
        if self.refreshing.contains_key(&key) {
            None
        } else {
            let cancel = CancelToken::new();
            self.refreshing.insert(
                key,
                DirectoryRefresh {
                    epoch: self.epoch,
                    cancel: cancel.clone(),
                },
            );
            Some((self.epoch, cancel))
        }
    }

    fn refresh_is_current(&self, storage_id: u64, parent: Option<u64>, epoch: u64) -> bool {
        self.epoch == epoch
            && self
                .refreshing
                .get(&DirectoryCacheKey::new(storage_id, parent))
                .is_some_and(|refresh| refresh.epoch == epoch && !refresh.cancel.is_cancelled())
    }

    fn finish_refresh(
        &mut self,
        storage_id: u64,
        parent: Option<u64>,
        epoch: u64,
        entries: Option<Vec<ObjectEntry>>,
        now: Instant,
    ) -> bool {
        let key = DirectoryCacheKey::new(storage_id, parent);
        let is_current = self.epoch == epoch
            && self
                .refreshing
                .get(&key)
                .is_some_and(|refresh| refresh.epoch == epoch && !refresh.cancel.is_cancelled());
        if !is_current {
            return false;
        }
        self.refreshing.remove(&key);
        let Some(entries) = entries else {
            return false;
        };
        let changed = self
            .listings
            .get(&key)
            .is_none_or(|cached| cached.entries != entries);
        self.store_listing_inner(key, entries, now);
        changed
    }

    fn can_count_validate(
        &self,
        storage_id: u64,
        parent: Option<u64>,
        epoch: u64,
        object_count: usize,
        now: Instant,
    ) -> bool {
        self.refresh_is_current(storage_id, parent, epoch)
            && self
                .listings
                .get(&DirectoryCacheKey::new(storage_id, parent))
                .is_some_and(|cached| {
                    cached.entries.len() == object_count
                        && now.saturating_duration_since(cached.fully_loaded_at)
                            < DIRECTORY_FULL_REFRESH_INTERVAL
                })
    }

    fn finish_count_validation(
        &mut self,
        storage_id: u64,
        parent: Option<u64>,
        epoch: u64,
        now: Instant,
    ) -> bool {
        let key = DirectoryCacheKey::new(storage_id, parent);
        if !self.refresh_is_current(storage_id, parent, epoch) {
            return false;
        }
        self.refreshing.remove(&key);
        if let Some(cached) = self.listings.get_mut(&key) {
            cached.loaded_at = now;
            true
        } else {
            false
        }
    }

    fn store_listing(
        &mut self,
        storage_id: u64,
        parent: Option<u64>,
        entries: Vec<ObjectEntry>,
        now: Instant,
    ) {
        let key = DirectoryCacheKey::new(storage_id, parent);
        self.cancel_refresh(key);
        self.store_listing_inner(key, entries, now);
    }

    fn store_listing_inner(
        &mut self,
        key: DirectoryCacheKey,
        entries: Vec<ObjectEntry>,
        now: Instant,
    ) {
        let current_handles: HashSet<u64> = entries.iter().map(|entry| entry.handle).collect();
        if let Some(previous) = self.listings.remove(&key) {
            for entry in previous.entries {
                if !current_handles.contains(&entry.handle) {
                    self.objects.remove(&(key.storage_id, entry.handle));
                }
            }
        }
        for entry in &entries {
            self.objects
                .insert((key.storage_id, entry.handle), entry.clone());
        }
        self.listings.insert(
            key,
            CachedListing {
                loaded_at: now,
                fully_loaded_at: now,
                entries,
            },
        );
    }

    fn store_object(&mut self, entry: ObjectEntry, now: Instant) {
        self.upsert_object(entry, now);
    }

    fn upsert_object(&mut self, entry: ObjectEntry, now: Instant) {
        self.cancel_refreshes();
        let storage_id = entry.storage_id;
        let handle = entry.handle;
        self.objects
            .retain(|(_, cached_handle), _| *cached_handle != handle);
        for (key, listing) in &mut self.listings {
            if key.storage_id == storage_id {
                let previous_len = listing.entries.len();
                listing.entries.retain(|cached| cached.handle != handle);
                if listing.entries.len() != previous_len {
                    listing.loaded_at = now;
                    listing.fully_loaded_at = now;
                }
            }
        }
        if let Some(listing) = self.listings.get_mut(&DirectoryCacheKey::new(
            storage_id,
            (entry.parent != 0).then_some(entry.parent),
        )) {
            listing.entries.push(entry.clone());
            listing.loaded_at = now;
            listing.fully_loaded_at = now;
        }
        self.objects.insert((storage_id, handle), entry);
    }

    fn invalidate_directory(&mut self, storage_id: u64, parent: Option<u64>) {
        let key = DirectoryCacheKey::new(storage_id, parent);
        self.cancel_refresh(key);
        self.listings.remove(&key);
    }

    fn invalidate_object(&mut self, storage_id: u64, handle: u64) {
        self.cancel_refreshes();
        let mut pending = vec![handle];
        while let Some(current) = pending.pop() {
            self.objects.remove(&(storage_id, current));
            if let Some(children) = self
                .listings
                .remove(&DirectoryCacheKey::new(storage_id, Some(current)))
            {
                pending.extend(children.entries.into_iter().map(|entry| entry.handle));
            }
            self.listings.retain(|key, listing| {
                key.storage_id != storage_id
                    || !listing.entries.iter().any(|entry| entry.handle == current)
            });
        }
    }

    fn remove_object(&mut self, storage_id: u64, handle: u64, now: Instant) {
        self.cancel_refreshes();
        let mut pending = vec![handle];
        while let Some(current) = pending.pop() {
            self.objects.remove(&(storage_id, current));
            if let Some(children) = self
                .listings
                .remove(&DirectoryCacheKey::new(storage_id, Some(current)))
            {
                pending.extend(children.entries.into_iter().map(|entry| entry.handle));
            }
            for (key, listing) in &mut self.listings {
                if key.storage_id == storage_id {
                    let previous_len = listing.entries.len();
                    listing.entries.retain(|entry| entry.handle != current);
                    if listing.entries.len() != previous_len {
                        listing.loaded_at = now;
                        listing.fully_loaded_at = now;
                    }
                }
            }
        }
    }

    fn remove_handle(&mut self, handle: u64, now: Instant) {
        let mut storage_ids: HashSet<u64> = self
            .objects
            .keys()
            .filter_map(|(storage_id, cached_handle)| {
                (*cached_handle == handle).then_some(*storage_id)
            })
            .collect();
        for (key, listing) in &self.listings {
            if listing.entries.iter().any(|entry| entry.handle == handle) {
                storage_ids.insert(key.storage_id);
            }
        }
        for storage_id in storage_ids {
            self.remove_object(storage_id, handle, now);
        }
    }

    fn clear(&mut self) {
        self.cancel_refreshes();
        self.listings.clear();
        self.objects.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn cancel_refresh(&mut self, key: DirectoryCacheKey) {
        if let Some(refresh) = self.refreshing.remove(&key) {
            refresh.cancel.cancel();
        }
    }

    fn cancel_refreshes(&mut self) {
        for refresh in self.refreshing.values() {
            refresh.cancel.cancel();
        }
        self.refreshing.clear();
    }
}

fn is_cache_fresh(loaded_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(loaded_at) <= DIRECTORY_CACHE_TTL
}

struct ConnectedDevice {
    key: String,
    physical_key: String,
    vendor_product_key: String,
    device: MtpDevice,
    gate: Semaphore,
    summary: RwLock<DeviceSummary>,
    storages: RwLock<HashMap<u64, Arc<Storage>>>,
    cache: RwLock<DeviceCache>,
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
        let storage_cache = storages
            .into_iter()
            .map(|storage| (storage.id().0, Arc::new(storage)))
            .collect();
        *self.storages.write().await = storage_cache;
        Ok(())
    }

    async fn storage(&self, storage_id: u64) -> std::result::Result<Arc<Storage>, MtpError> {
        if let Some(storage) = self.storages.read().await.get(&storage_id).cloned() {
            return Ok(storage);
        }
        let storage = Arc::new(self.device.storage(StorageId(storage_id)).await?);
        Ok(self
            .storages
            .write()
            .await
            .entry(storage_id)
            .or_insert_with(|| Arc::clone(&storage))
            .clone())
    }

    async fn cancel_background_refreshes(&self) {
        self.cache.write().await.cancel_refreshes();
    }

    async fn object_from_event(&self, handle: u64) -> Option<ObjectEntry> {
        let cached_storage =
            self.cache
                .read()
                .await
                .objects
                .iter()
                .find_map(|((storage_id, cached_handle), _)| {
                    (*cached_handle == handle).then_some(*storage_id)
                });
        let mut storage_ids: Vec<u64> = self
            .summary
            .read()
            .await
            .storages
            .iter()
            .map(|storage| storage.id)
            .collect();
        if let Some(cached_storage) = cached_storage
            && let Some(index) = storage_ids
                .iter()
                .position(|storage_id| *storage_id == cached_storage)
        {
            storage_ids.swap(0, index);
        }
        self.cancel_background_refreshes().await;
        let _permit = self.gate.acquire().await.ok()?;
        for storage_id in storage_ids {
            let Ok(storage) = self.storage(storage_id).await else {
                continue;
            };
            if let Ok(info) = storage.get_object_info(ObjectHandle(handle)).await {
                return Some(info.into());
            }
        }
        None
    }

    async fn apply_event(&self, event: DeviceEvent) -> bool {
        match event {
            DeviceEvent::ObjectAdded { handle } | DeviceEvent::ObjectInfoChanged { handle } => {
                if let Some(entry) = self.object_from_event(handle.0).await {
                    self.cache
                        .write()
                        .await
                        .upsert_object(entry, Instant::now());
                } else {
                    self.cache.write().await.clear();
                }
                true
            }
            DeviceEvent::ObjectRemoved { handle } => {
                self.cache
                    .write()
                    .await
                    .remove_handle(handle.0, Instant::now());
                true
            }
            DeviceEvent::StoreAdded { .. }
            | DeviceEvent::StoreRemoved { .. }
            | DeviceEvent::DeviceInfoChanged
            | DeviceEvent::DeviceReset => {
                self.cache.write().await.clear();
                true
            }
            DeviceEvent::StorageInfoChanged { .. } | DeviceEvent::Unknown { .. } => false,
        }
    }
}

fn spawn_device_event_listener(device: &Arc<ConnectedDevice>, last_change_millis: Arc<AtomicU64>) {
    let device = Arc::downgrade(device);
    tokio::spawn(async move {
        loop {
            let Some(connected) = device.upgrade() else {
                break;
            };
            let mtp = connected.device.clone();
            drop(connected);
            match tokio::time::timeout(EVENT_POLL_TIMEOUT, mtp.next_event()).await {
                Ok(Ok(event)) => {
                    let Some(connected) = device.upgrade() else {
                        break;
                    };
                    if connected.apply_event(event).await {
                        mark_timestamp_changed(&last_change_millis);
                    }
                }
                Ok(Err(MtpError::Timeout)) | Err(_) => {}
                Ok(Err(MtpError::Disconnected | MtpError::NoDevice)) => break,
                Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    });
}

fn spawn_directory_refresh(
    device: Arc<ConnectedDevice>,
    storage_id: u64,
    parent: Option<u64>,
    epoch: u64,
    cancel: CancelToken,
    last_change_millis: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = device.gate.acquire().await else {
            return;
        };
        if !device
            .cache
            .read()
            .await
            .refresh_is_current(storage_id, parent, epoch)
        {
            return;
        }
        let entries = match device.storage(storage_id).await {
            Ok(storage) => {
                match storage
                    .list_objects_stream_with_cancel(parent.map(ObjectHandle), Some(&cancel))
                    .await
                {
                    Ok(mut listing) => {
                        let now = Instant::now();
                        // Creating the stream has already fetched the handle count but not each
                        // object's metadata. Events keep names and metadata current, so an equal
                        // count is a cheap validation between periodic full scans.
                        let count_is_enough = device.cache.read().await.can_count_validate(
                            storage_id,
                            parent,
                            epoch,
                            listing.total(),
                            now,
                        );
                        if count_is_enough {
                            device
                                .cache
                                .write()
                                .await
                                .finish_count_validation(storage_id, parent, epoch, now);
                            return;
                        }
                        let mut entries = Vec::with_capacity(listing.total());
                        let mut failed = false;
                        while let Some(result) = listing.next().await {
                            match result {
                                Ok(object) => entries.push(object.into()),
                                Err(_) => {
                                    failed = true;
                                    break;
                                }
                            }
                        }
                        (!failed).then_some(entries)
                    }
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };
        let changed = device.cache.write().await.finish_refresh(
            storage_id,
            parent,
            epoch,
            entries,
            Instant::now(),
        );
        if changed {
            mark_timestamp_changed(&last_change_millis);
        }
    });
}

/// Owns all open MTP sessions and serializes operations per physical device.
#[derive(Clone)]
pub struct DeviceManager {
    devices: Arc<RwLock<HashMap<String, Arc<ConnectedDevice>>>>,
    open_failures: Arc<RwLock<HashMap<String, String>>>,
    last_error: Arc<RwLock<Option<String>>>,
    last_change_millis: Arc<AtomicU64>,
    next_generation: Arc<AtomicU64>,
    last_image_capture_reclaim: Arc<Mutex<Option<Instant>>>,
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
            last_image_capture_reclaim: Arc::new(Mutex::new(None)),
            logs,
        }
    }

    /// Enumerates USB devices, opens new sessions, and removes disconnected sessions.
    pub async fn refresh(&self) -> Result<Vec<DeviceSummary>> {
        let language = current_language();
        let candidates = tokio::task::spawn_blocking(MtpDevice::list_devices)
            .await
            .map_err(|error| Error::Operation(language.device_enumeration_failed(error)))??;
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
                    language.device_disconnected(&device.key),
                );
            }
        }

        let has_unconnected_candidate = {
            let connected = self.devices.read().await;
            candidates.iter().any(|candidate| {
                !connected
                    .values()
                    .any(|device| candidate_matches_connected(candidate, device))
            })
        };
        if has_unconnected_candidate {
            // Image Capture races MTP applications for Android's exclusive USB
            // interface on macOS. Reclaim it before opening the first session so
            // `icdd` cannot leave the device permanently unavailable in Finder.
            let _ = self.reclaim_image_capture().await;
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
            let mut opened = self.open_candidate(&candidate, key.clone()).await;
            if opened.as_ref().is_err_and(is_exclusive_error) && self.reclaim_image_capture().await
            {
                opened = self.open_candidate(&candidate, key.clone()).await;
            }
            match opened {
                Ok(connected) => {
                    self.open_failures.write().await.remove(&key);
                    self.logs.emit(
                        LogLevel::Info,
                        "mtp",
                        language.device_connected(
                            &connected.summary.read().await.manufacturer,
                            &connected.summary.read().await.model,
                        ),
                    );
                    let connected = Arc::new(connected);
                    spawn_device_event_listener(&connected, Arc::clone(&self.last_change_millis));
                    self.devices.write().await.insert(key, connected);
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
        let language = current_language();
        let devices: Vec<Arc<ConnectedDevice>> =
            self.devices.read().await.values().cloned().collect();
        for device in devices {
            if let Err(error) = device.refresh_storages().await {
                self.logs.emit(
                    LogLevel::Warn,
                    "mtp",
                    language.storage_refresh_failed(&device.key, error),
                );
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

    /// Drops cached object metadata after an explicit user refresh.
    pub async fn invalidate_caches(&self) {
        let devices: Vec<Arc<ConnectedDevice>> =
            self.devices.read().await.values().cloned().collect();
        for device in devices {
            device.cache.write().await.clear();
        }
        self.mark_changed();
    }

    #[must_use]
    pub fn last_change(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.last_change_millis.load(Ordering::Acquire))
    }

    fn mark_changed(&self) {
        mark_timestamp_changed(&self.last_change_millis);
    }

    async fn reclaim_image_capture(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            let mut last_reclaim = self.last_image_capture_reclaim.lock().await;
            let now = Instant::now();
            if last_reclaim.is_some_and(|last| {
                now.saturating_duration_since(last) < IMAGE_CAPTURE_RECLAIM_COOLDOWN
            }) {
                return false;
            }

            match terminate_image_capture_daemon().await {
                Ok(0) => false,
                Ok(count) => {
                    *last_reclaim = Some(now);
                    self.logs.emit(
                        LogLevel::Info,
                        "mtp",
                        current_language().image_capture_reclaimed(count),
                    );
                    // The process has exited, but IOKit releases its exclusive
                    // interface claim asynchronously. A short grace period avoids
                    // racing that teardown while still beating launchd's respawn.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    true
                }
                Err(error) => {
                    *last_reclaim = Some(now);
                    self.logs.emit(
                        LogLevel::Warn,
                        "mtp",
                        current_language().image_capture_reclaim_failed(error),
                    );
                    false
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    pub async fn list(
        &self,
        device_key: &str,
        storage_id: u64,
        parent: Option<u64>,
    ) -> Result<Vec<ObjectEntry>> {
        let device = self.get(device_key).await?;
        let (cached, refresh_epoch) = {
            let mut cache = device.cache.write().await;
            match cache.listing(storage_id, parent, Instant::now()) {
                Some((entries, true)) => (Some(entries), None),
                Some((entries, false)) => {
                    let epoch = cache.begin_refresh(storage_id, parent);
                    (Some(entries), epoch)
                }
                None => (None, None),
            }
        };
        if let Some((epoch, cancel)) = refresh_epoch {
            spawn_directory_refresh(
                Arc::clone(&device),
                storage_id,
                parent,
                epoch,
                cancel,
                Arc::clone(&self.last_change_millis),
            );
        }
        if let Some(entries) = cached {
            return Ok(entries);
        }
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        if let Some((entries, _)) =
            device
                .cache
                .read()
                .await
                .listing(storage_id, parent, Instant::now())
        {
            return Ok(entries);
        }
        let storage = device.storage(storage_id).await?;
        let entries: Vec<ObjectEntry> = storage
            .list_objects(parent.map(ObjectHandle))
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        device.cache.write().await.store_listing(
            storage_id,
            parent,
            entries.clone(),
            Instant::now(),
        );
        Ok(entries)
    }

    pub async fn metadata(
        &self,
        device_key: &str,
        storage_id: u64,
        handle: u64,
    ) -> Result<ObjectEntry> {
        let device = self.get(device_key).await?;
        if let Some(entry) = device.cache.read().await.object(storage_id, handle) {
            return Ok(entry);
        }
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        if let Some(entry) = device.cache.read().await.object(storage_id, handle) {
            return Ok(entry);
        }
        let storage = device.storage(storage_id).await?;
        let entry: ObjectEntry = storage.get_object_info(ObjectHandle(handle)).await?.into();
        device
            .cache
            .write()
            .await
            .store_object(entry.clone(), Instant::now());
        Ok(entry)
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
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
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
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
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
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
        let listing_parent = parent.map(ObjectHandle);
        let handle = storage
            .create_folder(mtp_write_parent(parent), name)
            .await?;
        let Ok(objects) = storage.list_objects(listing_parent).await else {
            device
                .cache
                .write()
                .await
                .invalidate_directory(storage_id, parent);
            self.mark_changed();
            return Ok(handle.0);
        };
        let canonical =
            find_uploaded_object(&objects, name, 0, Some(handle), Some(true)).unwrap_or(handle);
        let entries = objects.into_iter().map(Into::into).collect();
        device
            .cache
            .write()
            .await
            .store_listing(storage_id, parent, entries, Instant::now());
        self.mark_changed();
        Ok(canonical.0)
    }

    pub async fn delete(&self, device_key: &str, storage_id: u64, handle: u64) -> Result<()> {
        let device = self.get(device_key).await?;
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
        storage.delete(ObjectHandle(handle)).await?;
        device
            .cache
            .write()
            .await
            .remove_object(storage_id, handle, Instant::now());
        self.mark_changed();
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
                current_language().strings().rename_unsupported.into(),
            ));
        }
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
        storage.rename(ObjectHandle(handle), new_name).await?;
        device
            .cache
            .write()
            .await
            .invalidate_object(storage_id, handle);
        self.mark_changed();
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
            return Err(Error::Unsupported(
                current_language().strings().move_unsupported.into(),
            ));
        }
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(source_storage_id).await?;
        storage
            .move_object(
                ObjectHandle(handle),
                destination_parent.map_or(ObjectHandle::ALL, ObjectHandle),
                Some(StorageId(destination_storage_id)),
            )
            .await?;
        let mut cache = device.cache.write().await;
        cache.invalidate_object(source_storage_id, handle);
        cache.invalidate_directory(destination_storage_id, destination_parent);
        drop(cache);
        self.mark_changed();
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
                current_language().strings().upload_unsupported.into(),
            ));
        }
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
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
        let uploaded = match result {
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
                    Ok(handle.0)
                } else {
                    if let Some(partial) = upload_error.partial {
                        let _ = storage.delete(partial).await;
                    }
                    Err(Error::Mtp(upload_error.source))
                }
            }
        };
        if uploaded.is_ok() {
            device
                .cache
                .write()
                .await
                .invalidate_directory(storage_id, parent);
            self.mark_changed();
        }
        uploaded
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
        device.cancel_background_refreshes().await;
        let _permit = device
            .gate
            .acquire()
            .await
            .map_err(|_| Error::Disconnected)?;
        let storage = device.storage(storage_id).await?;
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
        let uploaded = match result {
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
                    Ok(handle.0)
                } else {
                    if let Some(partial) = upload_error.partial {
                        let _ = storage.delete(partial).await;
                    }
                    Err(Error::Mtp(upload_error.source))
                }
            }
        };
        if uploaded.is_ok() {
            device
                .cache
                .write()
                .await
                .invalidate_directory(storage_id, parent);
            self.mark_changed();
        }
        uploaded
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
        let strings = current_language().strings();
        let summary = DeviceSummary {
            key: key.clone(),
            manufacturer: nonempty(
                &info.manufacturer,
                candidate.manufacturer.as_deref(),
                strings.unknown,
            ),
            model: nonempty(
                &info.model,
                candidate.product.as_deref(),
                strings.mtp_device,
            ),
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
        let storages = storages
            .into_iter()
            .map(|storage| (storage.id().0, Arc::new(storage)))
            .collect();
        Ok(ConnectedDevice {
            key,
            physical_key: physical_device_key(candidate),
            vendor_product_key: vendor_product_key(candidate),
            device,
            gate: Semaphore::new(1),
            summary: RwLock::new(summary),
            storages: RwLock::new(storages),
            cache: RwLock::new(DeviceCache::default()),
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
    let language = current_language();
    let detail = error.to_string();
    if is_exclusive_error(error) {
        language.device_exclusively_held(key)
    } else {
        language.open_device_failed(key, detail)
    }
}

fn is_exclusive_error(error: &Error) -> bool {
    matches!(error, Error::Mtp(source) if source.is_exclusive_access())
}

#[cfg(target_os = "macos")]
async fn terminate_image_capture_daemon() -> std::io::Result<usize> {
    use nix::unistd::Uid;
    use std::process::Stdio;
    use tokio::process::Command;

    let output = Command::new("/bin/ps")
        .args(["-ww", "-axo", "pid=,uid=,command="])
        .output()
        .await?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let process_list = String::from_utf8_lossy(&output.stdout);
    let pids = image_capture_daemon_pids(&process_list, Uid::effective().as_raw());
    let mut terminated = Vec::with_capacity(pids.len());
    for pid in pids {
        let status = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
        if status.success() {
            terminated.push(pid);
        }
    }

    // Wait only for the instances we stopped. launchd may create a new `icdd`,
    // so MTPDrive must attempt its USB claim immediately after the old PIDs exit.
    for _ in 0..15 {
        if terminated.is_empty() {
            break;
        }
        let mut any_alive = false;
        for pid in &terminated {
            let status = Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await?;
            any_alive |= status.success();
        }
        if !any_alive {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(terminated.len())
}

fn image_capture_daemon_pids(process_list: &str, current_uid: u32) -> Vec<u32> {
    process_list
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse::<u32>().ok()?;
            let uid = fields.next()?.parse::<u32>().ok()?;
            let command = fields.collect::<Vec<_>>().join(" ");
            (uid == current_uid && command == IMAGE_CAPTURE_DAEMON_PATH).then_some(pid)
        })
        .collect()
}

fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn mark_timestamp_changed(timestamp: &AtomicU64) {
    let now = now_millis();
    let _ = timestamp.fetch_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
        Some(now.max(previous.saturating_add(1)))
    });
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
        format!("{} {id:08x}", current_language().strings().storage)
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
#[path = "../tests/unit/device.rs"]
mod tests;
