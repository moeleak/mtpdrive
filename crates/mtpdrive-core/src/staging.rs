use crate::device::DeviceManager;
use crate::i18n::current_language;
use crate::logs::LogStore;
use crate::model::LogLevel;
use crate::{Error, Result};
use bytes::Bytes;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

const FAILED_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RemoteIdentity {
    device_key: String,
    generation: u64,
    storage_id: u64,
    handle: u64,
}

#[derive(Debug)]
struct StagedState {
    device_key: String,
    generation: u64,
    storage_id: u64,
    parent: Option<u64>,
    name: String,
    origin: Option<u64>,
    remote: Option<u64>,
    path: PathBuf,
    local_only: bool,
    dirty: bool,
    revision: u64,
    modified: SystemTime,
}

/// Immutable state used by the NFS adapter without exposing internal locks.
#[derive(Debug, Clone)]
pub struct StagedSnapshot {
    pub id: Uuid,
    pub device_key: String,
    pub generation: u64,
    pub storage_id: u64,
    pub parent: Option<u64>,
    pub name: String,
    pub origin: Option<u64>,
    pub remote: Option<u64>,
    pub path: PathBuf,
    pub local_only: bool,
    pub dirty: bool,
    pub revision: u64,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Debug)]
struct StagedFile {
    id: Uuid,
    state: Mutex<StagedState>,
}

/// Random-write overlay that turns NFS writes into whole-object MTP uploads.
#[derive(Debug, Clone)]
pub struct StagingArea {
    root: PathBuf,
    files: Arc<RwLock<HashMap<Uuid, Arc<StagedFile>>>>,
    origins: Arc<RwLock<HashMap<RemoteIdentity, Uuid>>>,
    logs: LogStore,
}

impl StagingArea {
    pub fn new(root: impl Into<PathBuf>, logs: LogStore) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        cleanup_old_files(&root)?;
        Ok(Self {
            root,
            files: Arc::new(RwLock::new(HashMap::new())),
            origins: Arc::new(RwLock::new(HashMap::new())),
            logs,
        })
    }

    pub async fn create(
        &self,
        device_key: String,
        generation: u64,
        storage_id: u64,
        parent: Option<u64>,
        name: String,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let path = self.root.join(format!("{id}.data"));
        let file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .await?;
        file.sync_all().await?;
        let local_only = is_finder_metadata(&name);
        let staged = Arc::new(StagedFile {
            id,
            state: Mutex::new(StagedState {
                device_key,
                generation,
                storage_id,
                parent,
                name,
                origin: None,
                remote: None,
                path,
                local_only,
                dirty: true,
                revision: 1,
                modified: SystemTime::now(),
            }),
        });
        self.files.write().await.insert(id, staged);
        Ok(id)
    }

    pub async fn stage_existing(
        &self,
        manager: &DeviceManager,
        device_key: &str,
        generation: u64,
        storage_id: u64,
        handle: u64,
    ) -> Result<Uuid> {
        let identity = RemoteIdentity {
            device_key: device_key.to_owned(),
            generation,
            storage_id,
            handle,
        };
        if let Some(id) = self.origins.read().await.get(&identity).copied() {
            return Ok(id);
        }

        let metadata = manager.metadata(device_key, storage_id, handle).await?;
        if metadata.is_dir {
            return Err(Error::Unsupported(
                current_language()
                    .strings()
                    .stage_directory_unsupported
                    .into(),
            ));
        }
        let id = Uuid::new_v4();
        let path = self.root.join(format!("{id}.data"));
        manager
            .download_to(device_key, storage_id, handle, &path)
            .await?;
        let staged = Arc::new(StagedFile {
            id,
            state: Mutex::new(StagedState {
                device_key: device_key.to_owned(),
                generation,
                storage_id,
                parent: (metadata.parent != 0).then_some(metadata.parent),
                name: metadata.name,
                origin: Some(handle),
                remote: Some(handle),
                path,
                local_only: false,
                dirty: false,
                revision: 1,
                modified: SystemTime::now(),
            }),
        });

        let mut origins = self.origins.write().await;
        if let Some(existing) = origins.get(&identity).copied() {
            let _ = tokio::fs::remove_file(staged.state.lock().await.path.clone()).await;
            return Ok(existing);
        }
        origins.insert(identity, id);
        self.files.write().await.insert(id, staged);
        Ok(id)
    }

    pub async fn for_remote(
        &self,
        device_key: &str,
        generation: u64,
        storage_id: u64,
        handle: u64,
    ) -> Option<Uuid> {
        self.origins
            .read()
            .await
            .get(&RemoteIdentity {
                device_key: device_key.to_owned(),
                generation,
                storage_id,
                handle,
            })
            .copied()
    }

    pub async fn snapshot(&self, id: Uuid) -> Result<StagedSnapshot> {
        let file = self.get(id).await?;
        snapshot_file(&file).await
    }

    pub async fn all_for_parent(
        &self,
        device_key: &str,
        generation: u64,
        storage_id: u64,
        parent: Option<u64>,
    ) -> Vec<StagedSnapshot> {
        let files: Vec<Arc<StagedFile>> = self.files.read().await.values().cloned().collect();
        let mut result = Vec::new();
        for file in files {
            if let Ok(snapshot) = snapshot_file(&file).await
                && snapshot.device_key == device_key
                && snapshot.generation == generation
                && snapshot.storage_id == storage_id
                && snapshot.parent == parent
            {
                result.push(snapshot);
            }
        }
        result.sort_by(|left, right| left.name.cmp(&right.name));
        result
    }

    pub async fn find_child(
        &self,
        device_key: &str,
        generation: u64,
        storage_id: u64,
        parent: Option<u64>,
        name: &str,
    ) -> Option<Uuid> {
        self.all_for_parent(device_key, generation, storage_id, parent)
            .await
            .into_iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.id)
    }

    pub async fn read(&self, id: Uuid, offset: u64, count: u32) -> Result<Bytes> {
        let snapshot = self.snapshot(id).await?;
        let mut file = tokio::fs::File::open(snapshot.path).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        let mut buffer = vec![0_u8; usize::try_from(count).unwrap_or(usize::MAX)];
        let read = file.read(&mut buffer).await?;
        buffer.truncate(read);
        Ok(Bytes::from(buffer))
    }

    pub async fn write(&self, id: Uuid, offset: u64, data: &Bytes) -> Result<u32> {
        let file = self.get(id).await?;
        let mut state = file.state.lock().await;
        let mut handle = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&state.path)
            .await?;
        handle.seek(SeekFrom::Start(offset)).await?;
        handle.write_all(data).await?;
        handle.flush().await?;
        handle.sync_data().await?;
        state.dirty = true;
        state.revision = state.revision.saturating_add(1);
        state.modified = SystemTime::now();
        u32::try_from(data.len())
            .map_err(|_| Error::Operation(current_language().strings().write_too_large.into()))
    }

    pub async fn set_len(&self, id: Uuid, size: u64) -> Result<()> {
        let file = self.get(id).await?;
        let mut state = file.state.lock().await;
        if tokio::fs::metadata(&state.path).await?.len() == size {
            return Ok(());
        }
        let handle = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&state.path)
            .await?;
        handle.set_len(size).await?;
        handle.sync_data().await?;
        state.dirty = true;
        state.revision = state.revision.saturating_add(1);
        state.modified = SystemTime::now();
        Ok(())
    }

    pub async fn rename(&self, id: Uuid, parent: Option<u64>, name: String) -> Result<()> {
        let file = self.get(id).await?;
        let mut state = file.state.lock().await;
        state.parent = parent;
        state.name = name.clone();
        state.local_only = is_finder_metadata(&name);
        state.dirty = true;
        state.revision = state.revision.saturating_add(1);
        state.modified = SystemTime::now();
        Ok(())
    }

    pub async fn commit_if_revision(
        &self,
        id: Uuid,
        revision: u64,
        manager: &DeviceManager,
    ) -> Result<()> {
        let snapshot = self.snapshot(id).await?;
        if snapshot.dirty && snapshot.revision == revision {
            self.commit(id, manager).await?;
        }
        Ok(())
    }

    pub async fn commit_all(&self, manager: &DeviceManager) -> Result<()> {
        let ids: Vec<Uuid> = self.files.read().await.keys().copied().collect();
        let mut first_error = None;
        for id in ids {
            let dirty = self.snapshot(id).await.is_ok_and(|snapshot| snapshot.dirty);
            if dirty
                && let Err(error) = self.commit(id, manager).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub async fn commit(&self, id: Uuid, manager: &DeviceManager) -> Result<()> {
        let file = self.get(id).await?;
        let mut state = file.state.lock().await;
        if !state.dirty {
            return Ok(());
        }
        if state.local_only {
            state.dirty = false;
            return Ok(());
        }

        let old_handle = state.remote.or(state.origin);
        let new_handle = if let Some(old_handle) = old_handle {
            self.replace_remote(&state, old_handle, manager).await?
        } else {
            manager
                .upload_file(
                    &state.device_key,
                    state.storage_id,
                    state.parent,
                    &state.name,
                    &state.path,
                )
                .await?
        };
        state.remote = Some(new_handle);
        state.origin = Some(new_handle);
        state.dirty = false;
        state.modified = SystemTime::now();
        let identity = RemoteIdentity {
            device_key: state.device_key.clone(),
            generation: state.generation,
            storage_id: state.storage_id,
            handle: new_handle,
        };
        self.origins.write().await.insert(identity, id);
        self.logs.emit(
            LogLevel::Info,
            "transfer",
            current_language().uploaded(&state.name),
        );
        Ok(())
    }

    async fn replace_remote(
        &self,
        state: &StagedState,
        old_handle: u64,
        manager: &DeviceManager,
    ) -> Result<u64> {
        let temporary_name = format!(".mtpdrive-upload-{}", Uuid::new_v4().simple());
        let backup_name = format!(".mtpdrive-backup-{}", Uuid::new_v4().simple());
        let temporary_handle = manager
            .upload_file(
                &state.device_key,
                state.storage_id,
                state.parent,
                &temporary_name,
                &state.path,
            )
            .await?;

        match manager
            .rename(
                &state.device_key,
                state.storage_id,
                old_handle,
                &backup_name,
            )
            .await
        {
            Ok(()) => {
                if let Err(error) = manager
                    .rename(
                        &state.device_key,
                        state.storage_id,
                        temporary_handle,
                        &state.name,
                    )
                    .await
                {
                    let _ = manager
                        .rename(&state.device_key, state.storage_id, old_handle, &state.name)
                        .await;
                    let _ = manager
                        .delete(&state.device_key, state.storage_id, temporary_handle)
                        .await;
                    return Err(error);
                }
                let _ = manager
                    .delete(&state.device_key, state.storage_id, old_handle)
                    .await;
                Ok(temporary_handle)
            }
            Err(_) => {
                // Devices without rename cannot provide an atomic replacement. Keep the staged
                // source durable, remove the temporary upload, then replace the original.
                let _ = manager
                    .delete(&state.device_key, state.storage_id, temporary_handle)
                    .await;
                manager
                    .delete(&state.device_key, state.storage_id, old_handle)
                    .await?;
                manager
                    .upload_file(
                        &state.device_key,
                        state.storage_id,
                        state.parent,
                        &state.name,
                        &state.path,
                    )
                    .await
            }
        }
    }

    pub async fn remove(&self, id: Uuid, manager: &DeviceManager) -> Result<()> {
        let file = self.get(id).await?;
        let state = file.state.lock().await;
        if let Some(handle) = state.remote.or(state.origin)
            && !state.local_only
        {
            manager
                .delete(&state.device_key, state.storage_id, handle)
                .await?;
        }
        let path = state.path.clone();
        drop(state);
        self.files.write().await.remove(&id);
        let _ = tokio::fs::remove_file(path).await;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Arc<StagedFile>> {
        self.files
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(Error::NotFound)
    }
}

async fn snapshot_file(file: &StagedFile) -> Result<StagedSnapshot> {
    let state = file.state.lock().await;
    let size = tokio::fs::metadata(&state.path).await?.len();
    Ok(StagedSnapshot {
        id: file.id,
        device_key: state.device_key.clone(),
        generation: state.generation,
        storage_id: state.storage_id,
        parent: state.parent,
        name: state.name.clone(),
        origin: state.origin,
        remote: state.remote,
        path: state.path.clone(),
        local_only: state.local_only,
        dirty: state.dirty,
        revision: state.revision,
        size,
        modified: state.modified,
    })
}

fn cleanup_old_files(root: &Path) -> Result<()> {
    let now = SystemTime::now();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        if age.is_some_and(|age| age > FAILED_RETENTION) && metadata.is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[must_use]
pub fn is_finder_metadata(name: &str) -> bool {
    name == ".DS_Store"
        || name.starts_with("._")
        || matches!(name, ".Spotlight-V100" | ".Trashes" | ".fseventsd")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn random_writes_and_resize_work() {
        let root = tempfile::tempdir().expect("temporary directory");
        let area = StagingArea::new(root.path(), LogStore::memory_only()).expect("staging area");
        let id = area
            .create("phone".into(), 1, 2, None, "note.txt".into())
            .await
            .expect("create");
        area.write(id, 5, &Bytes::from_static(b"world"))
            .await
            .expect("write");
        area.write(id, 0, &Bytes::from_static(b"hello"))
            .await
            .expect("write");
        assert_eq!(
            area.read(id, 0, 10).await.expect("read"),
            Bytes::from_static(b"helloworld")
        );
        area.set_len(id, 5).await.expect("truncate");
        assert_eq!(area.snapshot(id).await.expect("snapshot").size, 5);
    }

    #[test]
    fn finder_metadata_is_recognized() {
        assert!(is_finder_metadata(".DS_Store"));
        assert!(is_finder_metadata("._photo.jpg"));
        assert!(!is_finder_metadata(".env"));
    }
}
