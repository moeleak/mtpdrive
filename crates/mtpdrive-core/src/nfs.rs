use crate::device::{DeviceManager, ObjectEntry};
use crate::logs::LogStore;
use crate::model::{DeviceSummary, LogLevel, StorageSummary};
use crate::staging::StagingArea;
use crate::{Error, Result};
use bytes::Bytes;
use fractal_nfs::nfs3_wire::{
    encode_access_ok, encode_commit_ok, encode_create_ok, encode_fsinfo_ok, encode_fsstat_ok,
    encode_getattr_ok, encode_lookup_ok, encode_mkdir_ok, encode_pathconf_ok, encode_read_ok,
    encode_readdir_ok, encode_readdirplus_ok, encode_remove_ok, encode_rename_ok,
    encode_setattr_ok, encode_write_ok,
};
use fractal_nfs::xdr::XdrWriter;
use fractal_nfs::{
    ACCESS3_DELETE, ACCESS3_EXECUTE, ACCESS3_EXTEND, ACCESS3_LOOKUP, ACCESS3_MODIFY, ACCESS3_READ,
    CreateHow3, Entry3, Entryplus3, FSF3_HOMOGENEOUS, Fattr3, Ftype3, Nfs3Filesystem, NfsFh3,
    NfsResult, Nfsstat3, Nfstime3, Sattr3, Specdata3, StableHow,
};
use nix::unistd::{Gid, Uid};
use parking_lot::RwLock as SyncRwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use time::{Date, Month, PrimitiveDateTime, Time};
use uuid::Uuid;

/// Eight-byte stable identifier used in NFS file handles.
pub const NFS_FSID: u64 = u64::from_be_bytes(*b"MTPDRIVE");

const ROOT_INODE: u64 = 1;
const FALLBACK_CAPACITY: u64 = 1024 * 1024 * 1024;
const MAX_IO_SIZE: u32 = 1024 * 1024;
const PREFERRED_IO_SIZE: u32 = 128 * 1024;

/// Stable identity passed between the NFS server and MTPDrive's backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeHandle {
    Root,
    Device {
        key: String,
        generation: u64,
    },
    Storage {
        device_key: String,
        generation: u64,
        storage_id: u64,
    },
    Object {
        device_key: String,
        generation: u64,
        storage_id: u64,
        object_id: u64,
    },
    Staged(Uuid),
}

#[derive(Debug)]
struct DirectoryContext {
    device_key: String,
    generation: u64,
    storage_id: u64,
    parent: Option<u64>,
    writable: bool,
}

#[derive(Debug)]
struct HandleTable {
    next: AtomicU64,
    by_inode: SyncRwLock<HashMap<u64, NodeHandle>>,
    by_handle: SyncRwLock<HashMap<NodeHandle, u64>>,
}

impl HandleTable {
    fn new() -> Self {
        let mut by_inode = HashMap::new();
        by_inode.insert(ROOT_INODE, NodeHandle::Root);
        let mut by_handle = HashMap::new();
        by_handle.insert(NodeHandle::Root, ROOT_INODE);
        Self {
            next: AtomicU64::new(ROOT_INODE + 1),
            by_inode: SyncRwLock::new(by_inode),
            by_handle: SyncRwLock::new(by_handle),
        }
    }

    fn intern(&self, handle: NodeHandle) -> u64 {
        if let Some(inode) = self.by_handle.read().get(&handle).copied() {
            return inode;
        }

        let mut by_handle = self.by_handle.write();
        if let Some(inode) = by_handle.get(&handle).copied() {
            return inode;
        }
        let inode = self.next.fetch_add(1, Ordering::Relaxed);
        by_handle.insert(handle.clone(), inode);
        self.by_inode.write().insert(inode, handle);
        inode
    }

    fn resolve(&self, fh: &NfsFh3) -> std::result::Result<NodeHandle, Nfsstat3> {
        if fh.data.len() != 16 || fh.fsid() != NFS_FSID {
            return Err(Nfsstat3::Badhandle);
        }
        self.by_inode
            .read()
            .get(&fh.ino())
            .cloned()
            .ok_or(Nfsstat3::Stale)
    }
}

/// NFSv3 filesystem that projects all connected MTP devices into one volume.
#[derive(Debug, Clone)]
pub struct MtpNfsFileSystem {
    manager: DeviceManager,
    staging: StagingArea,
    handles: Arc<HandleTable>,
    logs: LogStore,
    uid: u32,
    gid: u32,
    cookie_verifier: [u8; 8],
    write_verifier: [u8; 8],
}

impl MtpNfsFileSystem {
    pub fn new(
        manager: DeviceManager,
        staging: StagingArea,
        sidecar_root: impl Into<PathBuf>,
        logs: LogStore,
    ) -> Result<Self> {
        // NFSv3 has no named-attribute protocol. Finder stores resource forks in
        // AppleDouble `._*` files, which the staging layer deliberately keeps local.
        std::fs::create_dir_all(sidecar_root.into())?;
        let started = SystemTime::now();
        let started_millis = started
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let mut cookie_verifier = [0_u8; 8];
        cookie_verifier.copy_from_slice(&started_millis.to_be_bytes()[8..]);
        let mut write_verifier = cookie_verifier;
        write_verifier[0] ^= 0xa5;
        Ok(Self {
            manager,
            staging,
            handles: Arc::new(HandleTable::new()),
            logs,
            uid: Uid::current().as_raw(),
            gid: Gid::current().as_raw(),
            cookie_verifier,
            write_verifier,
        })
    }

    #[must_use]
    pub fn manager(&self) -> &DeviceManager {
        &self.manager
    }

    fn file_handle(&self, handle: NodeHandle) -> NfsFh3 {
        NfsFh3::new(self.handles.intern(handle), NFS_FSID)
    }

    fn resolve(&self, fh: &NfsFh3) -> std::result::Result<NodeHandle, Nfsstat3> {
        self.handles.resolve(fh)
    }

    async fn named_devices(&self) -> NfsFsResult<Vec<(String, DeviceSummary)>> {
        let summaries = self.manager.summaries().await.map_err(map_error)?;
        let mut counts = HashMap::<String, usize>::new();
        for summary in &summaries {
            *counts.entry(safe_component(&summary.model)).or_default() += 1;
        }
        Ok(summaries
            .into_iter()
            .map(|summary| {
                let base = safe_component(&summary.model);
                let name = if counts.get(&base).copied().unwrap_or(0) > 1 {
                    format!("{base} ({})", serial_suffix(&summary.serial))
                } else {
                    base
                };
                (name, summary)
            })
            .collect())
    }

    async fn named_storages(&self, device_key: &str) -> NfsFsResult<Vec<(String, StorageSummary)>> {
        let summary = self.manager.summary(device_key).await.map_err(map_error)?;
        let mut counts = HashMap::<String, usize>::new();
        for storage in &summary.storages {
            *counts.entry(safe_component(&storage.name)).or_default() += 1;
        }
        Ok(summary
            .storages
            .into_iter()
            .map(|storage| {
                let base = safe_component(&storage.name);
                let name = if counts.get(&base).copied().unwrap_or(0) > 1 {
                    format!("{base} ({:08x})", storage.id)
                } else {
                    base
                };
                (name, storage)
            })
            .collect())
    }

    async fn directory_context(&self, handle: &NodeHandle) -> NfsFsResult<DirectoryContext> {
        match handle {
            NodeHandle::Storage {
                device_key,
                generation,
                storage_id,
            } => {
                let summary = self.manager.summary(device_key).await.map_err(map_error)?;
                ensure_generation(&summary, *generation)?;
                let storage = summary
                    .storages
                    .iter()
                    .find(|storage| storage.id == *storage_id)
                    .ok_or(Nfsstat3::Stale)?;
                Ok(DirectoryContext {
                    device_key: device_key.clone(),
                    generation: *generation,
                    storage_id: *storage_id,
                    parent: None,
                    writable: storage.writable && summary.writable,
                })
            }
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                let summary = self.manager.summary(device_key).await.map_err(map_error)?;
                ensure_generation(&summary, *generation)?;
                let object = self
                    .manager
                    .metadata(device_key, *storage_id, *object_id)
                    .await
                    .map_err(map_error)?;
                if !object.is_dir {
                    return Err(Nfsstat3::Notdir);
                }
                let writable = summary.writable
                    && summary
                        .storages
                        .iter()
                        .find(|storage| storage.id == *storage_id)
                        .is_some_and(|storage| storage.writable);
                Ok(DirectoryContext {
                    device_key: device_key.clone(),
                    generation: *generation,
                    storage_id: *storage_id,
                    parent: Some(*object_id),
                    writable,
                })
            }
            _ => Err(Nfsstat3::Notdir),
        }
    }

    async fn lookup_in_directory(
        &self,
        context: &DirectoryContext,
        name: &str,
    ) -> NfsFsResult<NodeHandle> {
        if let Some(id) = self
            .staging
            .find_child(
                &context.device_key,
                context.generation,
                context.storage_id,
                context.parent,
                name,
            )
            .await
        {
            let snapshot = self.staging.snapshot(id).await.map_err(map_error)?;
            return Ok(snapshot.remote.or(snapshot.origin).map_or(
                NodeHandle::Staged(id),
                |object_id| NodeHandle::Object {
                    device_key: context.device_key.clone(),
                    generation: context.generation,
                    storage_id: context.storage_id,
                    object_id,
                },
            ));
        }

        let objects = self
            .manager
            .list(&context.device_key, context.storage_id, context.parent)
            .await
            .map_err(map_error)?;
        let object = objects
            .into_iter()
            .find(|object| object.name == name)
            .ok_or(Nfsstat3::Noent)?;
        Ok(NodeHandle::Object {
            device_key: context.device_key.clone(),
            generation: context.generation,
            storage_id: context.storage_id,
            object_id: object.handle,
        })
    }

    async fn lookup_handle(&self, parent: &NodeHandle, name: &str) -> NfsFsResult<NodeHandle> {
        if name == "." {
            return Ok(parent.clone());
        }
        if name == ".." {
            return Ok(self
                .parent_handle(parent)
                .await?
                .unwrap_or(NodeHandle::Root));
        }
        match parent {
            NodeHandle::Root => self
                .named_devices()
                .await?
                .into_iter()
                .find(|(entry_name, _)| entry_name == name)
                .map(|(_, summary)| NodeHandle::Device {
                    key: summary.key,
                    generation: summary.generation,
                })
                .ok_or(Nfsstat3::Noent),
            NodeHandle::Device { key, generation } => self
                .named_storages(key)
                .await?
                .into_iter()
                .find(|(entry_name, _)| entry_name == name)
                .map(|(_, storage)| NodeHandle::Storage {
                    device_key: key.clone(),
                    generation: *generation,
                    storage_id: storage.id,
                })
                .ok_or(Nfsstat3::Noent),
            NodeHandle::Storage { .. } | NodeHandle::Object { .. } => {
                let context = self.directory_context(parent).await?;
                self.lookup_in_directory(&context, name).await
            }
            NodeHandle::Staged(_) => Err(Nfsstat3::Notdir),
        }
    }

    async fn attrs(&self, handle: &NodeHandle) -> NfsFsResult<Fattr3> {
        let inode = self.handles.intern(handle.clone());
        let virtual_timestamp = timestamp_from_system(self.manager.last_change());
        match handle {
            NodeHandle::Root => Ok(directory_attrs(
                inode,
                self.uid,
                self.gid,
                virtual_timestamp,
            )),
            NodeHandle::Device { key, generation } => {
                let summary = self.manager.summary(key).await.map_err(map_error)?;
                ensure_generation(&summary, *generation)?;
                Ok(directory_attrs(
                    inode,
                    self.uid,
                    self.gid,
                    virtual_timestamp,
                ))
            }
            NodeHandle::Storage {
                device_key,
                generation,
                storage_id,
            } => {
                let summary = self.manager.summary(device_key).await.map_err(map_error)?;
                ensure_generation(&summary, *generation)?;
                if !summary
                    .storages
                    .iter()
                    .any(|storage| storage.id == *storage_id)
                {
                    return Err(Nfsstat3::Stale);
                }
                Ok(directory_attrs(
                    inode,
                    self.uid,
                    self.gid,
                    virtual_timestamp,
                ))
            }
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                let summary = self.manager.summary(device_key).await.map_err(map_error)?;
                ensure_generation(&summary, *generation)?;
                if let Some(staged) = self
                    .staging
                    .for_remote(device_key, *generation, *storage_id, *object_id)
                    .await
                {
                    return self.staged_attrs(inode, staged).await;
                }
                let object = self
                    .manager
                    .metadata(device_key, *storage_id, *object_id)
                    .await
                    .map_err(map_error)?;
                Ok(object_attrs(inode, &object, self.uid, self.gid))
            }
            NodeHandle::Staged(id) => self.staged_attrs(inode, *id).await,
        }
    }

    async fn staged_attrs(&self, inode: u64, id: Uuid) -> NfsFsResult<Fattr3> {
        let snapshot = self.staging.snapshot(id).await.map_err(map_error)?;
        let timestamp = timestamp_from_system(snapshot.modified);
        Ok(Fattr3 {
            ftype: Ftype3::Reg,
            mode: 0o644,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            size: snapshot.size,
            used: snapshot.size,
            rdev: Specdata3::default(),
            fsid: NFS_FSID,
            fileid: inode,
            atime: timestamp,
            mtime: timestamp,
            ctime: timestamp,
        })
    }

    async fn parent_handle(&self, handle: &NodeHandle) -> NfsFsResult<Option<NodeHandle>> {
        match handle {
            NodeHandle::Root => Ok(None),
            NodeHandle::Device { .. } => Ok(Some(NodeHandle::Root)),
            NodeHandle::Storage {
                device_key,
                generation,
                ..
            } => Ok(Some(NodeHandle::Device {
                key: device_key.clone(),
                generation: *generation,
            })),
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                let object = self
                    .manager
                    .metadata(device_key, *storage_id, *object_id)
                    .await
                    .map_err(map_error)?;
                if object.parent == 0 {
                    Ok(Some(NodeHandle::Storage {
                        device_key: device_key.clone(),
                        generation: *generation,
                        storage_id: *storage_id,
                    }))
                } else {
                    Ok(Some(NodeHandle::Object {
                        device_key: device_key.clone(),
                        generation: *generation,
                        storage_id: *storage_id,
                        object_id: object.parent,
                    }))
                }
            }
            NodeHandle::Staged(id) => {
                let snapshot = self.staging.snapshot(*id).await.map_err(map_error)?;
                Ok(Some(snapshot.parent.map_or(
                    NodeHandle::Storage {
                        device_key: snapshot.device_key.clone(),
                        generation: snapshot.generation,
                        storage_id: snapshot.storage_id,
                    },
                    |object_id| NodeHandle::Object {
                        device_key: snapshot.device_key.clone(),
                        generation: snapshot.generation,
                        storage_id: snapshot.storage_id,
                        object_id,
                    },
                )))
            }
        }
    }

    async fn stage_for_object(&self, handle: &NodeHandle) -> NfsFsResult<Uuid> {
        let NodeHandle::Object {
            device_key,
            generation,
            storage_id,
            object_id,
        } = handle
        else {
            return Err(Nfsstat3::Inval);
        };
        if let Some(id) = self
            .staging
            .for_remote(device_key, *generation, *storage_id, *object_id)
            .await
        {
            return Ok(id);
        }
        self.staging
            .stage_existing(
                &self.manager,
                device_key,
                *generation,
                *storage_id,
                *object_id,
            )
            .await
            .map_err(map_error)
    }

    async fn staged_id(&self, handle: &NodeHandle) -> Option<Uuid> {
        match handle {
            NodeHandle::Staged(id) => Some(*id),
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                self.staging
                    .for_remote(device_key, *generation, *storage_id, *object_id)
                    .await
            }
            _ => None,
        }
    }

    async fn remove_handle(&self, handle: NodeHandle) -> NfsFsResult<()> {
        match handle {
            NodeHandle::Staged(id) => self
                .staging
                .remove(id, &self.manager)
                .await
                .map_err(map_error),
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                if let Some(id) = self
                    .staging
                    .for_remote(&device_key, generation, storage_id, object_id)
                    .await
                {
                    return self
                        .staging
                        .remove(id, &self.manager)
                        .await
                        .map_err(map_error);
                }
                let metadata = self
                    .manager
                    .metadata(&device_key, storage_id, object_id)
                    .await
                    .map_err(map_error)?;
                if metadata.is_dir
                    && !self
                        .manager
                        .list(&device_key, storage_id, Some(object_id))
                        .await
                        .map_err(map_error)?
                        .is_empty()
                {
                    return Err(Nfsstat3::Notempty);
                }
                self.manager
                    .delete(&device_key, storage_id, object_id)
                    .await
                    .map_err(map_error)
            }
            _ => Err(Nfsstat3::Acces),
        }
    }

    async fn is_writable(&self, handle: &NodeHandle) -> NfsFsResult<bool> {
        match handle {
            NodeHandle::Staged(id) => {
                self.staging.snapshot(*id).await.map_err(map_error)?;
                Ok(true)
            }
            NodeHandle::Storage { .. } | NodeHandle::Object { .. } => {
                let (device_key, generation, storage_id) = match handle {
                    NodeHandle::Storage {
                        device_key,
                        generation,
                        storage_id,
                    }
                    | NodeHandle::Object {
                        device_key,
                        generation,
                        storage_id,
                        ..
                    } => (device_key, *generation, *storage_id),
                    _ => unreachable!(),
                };
                let summary = self.manager.summary(device_key).await.map_err(map_error)?;
                ensure_generation(&summary, generation)?;
                Ok(summary.writable
                    && summary
                        .storages
                        .iter()
                        .find(|storage| storage.id == storage_id)
                        .is_some_and(|storage| storage.writable))
            }
            _ => Ok(false),
        }
    }

    async fn is_directory(&self, handle: &NodeHandle) -> NfsFsResult<bool> {
        Ok(self.attrs(handle).await?.ftype == Ftype3::Dir)
    }

    async fn directory_entries(&self, dir: &NodeHandle) -> NfsFsResult<Vec<(String, NodeHandle)>> {
        let mut handles: Vec<(String, NodeHandle)> = match dir {
            NodeHandle::Root => self
                .named_devices()
                .await?
                .into_iter()
                .map(|(name, summary)| {
                    (
                        name,
                        NodeHandle::Device {
                            key: summary.key,
                            generation: summary.generation,
                        },
                    )
                })
                .collect(),
            NodeHandle::Device { key, generation } => self
                .named_storages(key)
                .await?
                .into_iter()
                .map(|(name, storage)| {
                    (
                        name,
                        NodeHandle::Storage {
                            device_key: key.clone(),
                            generation: *generation,
                            storage_id: storage.id,
                        },
                    )
                })
                .collect(),
            NodeHandle::Storage { .. } | NodeHandle::Object { .. } => {
                let context = self.directory_context(dir).await?;
                let objects = self
                    .manager
                    .list(&context.device_key, context.storage_id, context.parent)
                    .await
                    .map_err(map_error)?;
                let mut entries: Vec<(String, NodeHandle)> = objects
                    .into_iter()
                    .map(|object| {
                        (
                            object.name,
                            NodeHandle::Object {
                                device_key: context.device_key.clone(),
                                generation: context.generation,
                                storage_id: context.storage_id,
                                object_id: object.handle,
                            },
                        )
                    })
                    .collect();
                let staged = self
                    .staging
                    .all_for_parent(
                        &context.device_key,
                        context.generation,
                        context.storage_id,
                        context.parent,
                    )
                    .await;
                for snapshot in staged {
                    if snapshot.origin.is_none() {
                        entries.retain(|(name, _)| name != &snapshot.name);
                        entries.push((snapshot.name, NodeHandle::Staged(snapshot.id)));
                    }
                }
                entries
            }
            NodeHandle::Staged(_) => return Err(Nfsstat3::Notdir),
        };
        handles.sort_by(|left, right| left.0.cmp(&right.0));
        let parent = self
            .parent_handle(dir)
            .await?
            .unwrap_or_else(|| dir.clone());
        handles.insert(0, ("..".to_owned(), parent));
        handles.insert(0, (".".to_owned(), dir.clone()));
        Ok(handles)
    }

    fn validate_cookie(&self, cookie: u64, verifier: [u8; 8]) -> NfsFsResult<()> {
        if cookie != 0 && verifier != self.cookie_verifier {
            return Err(Nfsstat3::BadCookie);
        }
        Ok(())
    }

    async fn read_bytes(
        &self,
        handle: &NodeHandle,
        offset: u64,
        count: u32,
    ) -> NfsFsResult<(Bytes, u64)> {
        match handle {
            NodeHandle::Staged(id) => {
                let snapshot = self.staging.snapshot(*id).await.map_err(map_error)?;
                let data = self
                    .staging
                    .read(*id, offset, count)
                    .await
                    .map_err(map_error)?;
                Ok((data, snapshot.size))
            }
            NodeHandle::Object {
                device_key,
                generation,
                storage_id,
                object_id,
            } => {
                if let Some(id) = self
                    .staging
                    .for_remote(device_key, *generation, *storage_id, *object_id)
                    .await
                {
                    let snapshot = self.staging.snapshot(id).await.map_err(map_error)?;
                    let data = self
                        .staging
                        .read(id, offset, count)
                        .await
                        .map_err(map_error)?;
                    return Ok((data, snapshot.size));
                }
                let metadata = self
                    .manager
                    .metadata(device_key, *storage_id, *object_id)
                    .await
                    .map_err(map_error)?;
                if metadata.is_dir {
                    return Err(Nfsstat3::Isdir);
                }
                let data = self
                    .manager
                    .read(device_key, *storage_id, *object_id, offset, count)
                    .await
                    .map_err(map_error)?;
                Ok((data, metadata.size))
            }
            _ => Err(Nfsstat3::Isdir),
        }
    }

    async fn commit_handle(&self, handle: &NodeHandle) -> NfsFsResult<()> {
        if let Some(id) = self.staged_id(handle).await {
            self.logs
                .emit(LogLevel::Debug, "nfs", "committing staged file");
            self.staging
                .commit(id, &self.manager)
                .await
                .map_err(map_error)?;
        }
        Ok(())
    }

    async fn schedule_commit(&self, id: Uuid) -> NfsFsResult<()> {
        let revision = self.staging.snapshot(id).await.map_err(map_error)?.revision;
        let staging = self.staging.clone();
        let manager = self.manager.clone();
        let logs = self.logs.clone();
        tokio::spawn(async move {
            // Give Finder enough time to send the data after CREATE/SETATTR.
            // A shorter delay can upload the pre-sized placeholder first and
            // then immediately replace it when the WRITE arrives.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if let Err(error) = staging.commit_if_revision(id, revision, &manager).await {
                logs.emit(
                    LogLevel::Warn,
                    "transfer",
                    format!("后台提交暂存文件失败：{error}"),
                );
            }
        });
        Ok(())
    }

    pub async fn flush_dirty(&self) -> Result<()> {
        self.staging.commit_all(&self.manager).await
    }

    async fn rename_handle(
        &self,
        source: NodeHandle,
        source_context: &DirectoryContext,
        destination_context: &DirectoryContext,
        from_name: &str,
        to_name: &str,
    ) -> NfsFsResult<()> {
        match source {
            NodeHandle::Staged(id) => self
                .staging
                .rename(id, destination_context.parent, to_name.to_owned())
                .await
                .map_err(map_error),
            NodeHandle::Object {
                generation,
                storage_id,
                object_id,
                ..
            } => {
                if let Some(id) = self
                    .staging
                    .for_remote(
                        &source_context.device_key,
                        generation,
                        storage_id,
                        object_id,
                    )
                    .await
                {
                    self.staging
                        .rename(id, destination_context.parent, to_name.to_owned())
                        .await
                        .map_err(map_error)?;
                    return Ok(());
                }
                if storage_id != destination_context.storage_id
                    || source_context.parent != destination_context.parent
                {
                    self.manager
                        .move_object(
                            &source_context.device_key,
                            storage_id,
                            object_id,
                            destination_context.storage_id,
                            destination_context.parent,
                        )
                        .await
                        .map_err(map_error)?;
                }
                if from_name != to_name {
                    self.manager
                        .rename(
                            &source_context.device_key,
                            destination_context.storage_id,
                            object_id,
                            to_name,
                        )
                        .await
                        .map_err(map_error)?;
                }
                Ok(())
            }
            _ => Err(Nfsstat3::Acces),
        }
    }
}

impl Nfs3Filesystem for MtpNfsFileSystem {
    fn getattr(&self, fh: &NfsFh3, w: &mut XdrWriter) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let attrs = self.attrs(&handle).await?;
            encode_getattr_ok(w, &attrs);
            Ok(())
        }
    }

    fn setattr(
        &self,
        fh: &NfsFh3,
        attrs: &Sattr3,
        guard_ctime: Option<Nfstime3>,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let mut pending_commit = None;
            if let Some(guard) = guard_ctime {
                let current = self.attrs(&handle).await?;
                if current.ctime.seconds != guard.seconds
                    || current.ctime.nseconds != guard.nseconds
                {
                    return Err(Nfsstat3::NotSync);
                }
            }
            if let Some(size) = attrs.size {
                let id = match &handle {
                    NodeHandle::Staged(id) => *id,
                    NodeHandle::Object { .. } => self.stage_for_object(&handle).await?,
                    _ => return Err(Nfsstat3::Isdir),
                };
                self.staging.set_len(id, size).await.map_err(map_error)?;
                if self
                    .staging
                    .snapshot(id)
                    .await
                    .map_err(map_error)?
                    .origin
                    .is_some()
                {
                    pending_commit = Some(id);
                }
            }
            let current = self.attrs(&handle).await?;
            if let Some(id) = pending_commit {
                // Start the debounce only after attribute resolution, so the
                // client can receive SETATTR's reply and issue its WRITE.
                self.schedule_commit(id).await?;
            }
            encode_setattr_ok(w, &current);
            Ok(())
        }
    }

    fn lookup(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let parent = self.resolve(dir_fh)?;
            let child = self.lookup_handle(&parent, name).await?;
            let child_fh = self.file_handle(child.clone());
            let attrs = self.attrs(&child).await?;
            let dir_attrs = self.attrs(&parent).await.ok();
            encode_lookup_ok(w, &child_fh, &attrs, dir_attrs.as_ref());
            Ok(())
        }
    }

    fn access(
        &self,
        fh: &NfsFh3,
        requested: u32,
        _uid: u32,
        _gid: u32,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let mut granted = ACCESS3_READ;
            if self.is_directory(&handle).await? {
                granted |= ACCESS3_LOOKUP | ACCESS3_EXECUTE;
            }
            if self.is_writable(&handle).await? {
                granted |= ACCESS3_MODIFY | ACCESS3_EXTEND | ACCESS3_DELETE;
            }
            let attrs = self.attrs(&handle).await?;
            encode_access_ok(w, &attrs, requested & granted);
            Ok(())
        }
    }

    fn read(
        &self,
        fh: &NfsFh3,
        offset: u64,
        count: u32,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let count = count.min(MAX_IO_SIZE);
            let (data, size) = self.read_bytes(&handle, offset, count).await?;
            let attrs = self.attrs(&handle).await?;
            let eof = offset.saturating_add(data.len() as u64) >= size;
            encode_read_ok(w, &attrs, &data, eof);
            Ok(())
        }
    }

    fn write(
        &self,
        fh: &NfsFh3,
        offset: u64,
        data: &[u8],
        stable: StableHow,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            if data.len() > MAX_IO_SIZE as usize {
                return Err(Nfsstat3::Fbig);
            }
            let id = match &handle {
                NodeHandle::Staged(id) => *id,
                NodeHandle::Object { .. } => self.stage_for_object(&handle).await?,
                _ => return Err(Nfsstat3::Isdir),
            };
            let data = Bytes::copy_from_slice(data);
            let written = self
                .staging
                .write(id, offset, &data)
                .await
                .map_err(map_error)?;
            let committed = if stable == StableHow::Unstable {
                self.schedule_commit(id).await?;
                StableHow::Unstable
            } else {
                self.commit_handle(&handle).await?;
                stable
            };
            let attrs = self.attrs(&handle).await?;
            encode_write_ok(w, &attrs, written, committed, &self.write_verifier);
            Ok(())
        }
    }

    fn create(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        how: &CreateHow3,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            validate_name(name)?;
            let parent = self.resolve(dir_fh)?;
            let context = self.directory_context(&parent).await?;
            if !context.writable {
                return Err(Nfsstat3::Rofs);
            }

            let requested_attrs = match how {
                CreateHow3::Unchecked(attrs) | CreateHow3::Guarded(attrs) => Some(attrs),
                CreateHow3::Exclusive(_) => None,
            };
            let child = if let Ok(existing) = self.lookup_in_directory(&context, name).await {
                if !matches!(how, CreateHow3::Unchecked(_)) {
                    return Err(Nfsstat3::Exist);
                }
                if self.is_directory(&existing).await? {
                    return Err(Nfsstat3::Isdir);
                }
                if let Some(size) = requested_attrs.and_then(|attrs| attrs.size) {
                    let id = match &existing {
                        NodeHandle::Staged(id) => *id,
                        NodeHandle::Object { .. } => self.stage_for_object(&existing).await?,
                        _ => return Err(Nfsstat3::Inval),
                    };
                    self.staging.set_len(id, size).await.map_err(map_error)?;
                }
                existing
            } else {
                let id = self
                    .staging
                    .create(
                        context.device_key,
                        context.generation,
                        context.storage_id,
                        context.parent,
                        name.to_owned(),
                    )
                    .await
                    .map_err(map_error)?;
                if let Some(size) = requested_attrs.and_then(|attrs| attrs.size) {
                    self.staging.set_len(id, size).await.map_err(map_error)?;
                }
                NodeHandle::Staged(id)
            };
            let child_fh = self.file_handle(child.clone());
            let child_attrs = self.attrs(&child).await?;
            let parent_attrs = self.attrs(&parent).await.ok();
            encode_create_ok(w, &child_fh, &child_attrs, parent_attrs.as_ref());
            Ok(())
        }
    }

    fn mkdir(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        _attrs: &Sattr3,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            validate_name(name)?;
            let parent = self.resolve(dir_fh)?;
            let context = self.directory_context(&parent).await?;
            if !context.writable {
                return Err(Nfsstat3::Rofs);
            }
            if self.lookup_in_directory(&context, name).await.is_ok() {
                return Err(Nfsstat3::Exist);
            }
            let object_id = self
                .manager
                .create_folder(
                    &context.device_key,
                    context.storage_id,
                    context.parent,
                    name,
                )
                .await
                .map_err(map_error)?;
            let child = NodeHandle::Object {
                device_key: context.device_key,
                generation: context.generation,
                storage_id: context.storage_id,
                object_id,
            };
            let child_fh = self.file_handle(child.clone());
            let child_attrs = self.attrs(&child).await?;
            let parent_attrs = self.attrs(&parent).await.ok();
            encode_mkdir_ok(w, &child_fh, &child_attrs, parent_attrs.as_ref());
            Ok(())
        }
    }

    fn remove(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let parent = self.resolve(dir_fh)?;
            let context = self.directory_context(&parent).await?;
            if !context.writable {
                return Err(Nfsstat3::Rofs);
            }
            let child = self.lookup_in_directory(&context, name).await?;
            if self.is_directory(&child).await? {
                return Err(Nfsstat3::Isdir);
            }
            self.remove_handle(child).await?;
            let parent_attrs = self.attrs(&parent).await.ok();
            encode_remove_ok(w, parent_attrs.as_ref());
            Ok(())
        }
    }

    fn rmdir(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let parent = self.resolve(dir_fh)?;
            let context = self.directory_context(&parent).await?;
            if !context.writable {
                return Err(Nfsstat3::Rofs);
            }
            let child = self.lookup_in_directory(&context, name).await?;
            if !self.is_directory(&child).await? {
                return Err(Nfsstat3::Notdir);
            }
            self.remove_handle(child).await?;
            let parent_attrs = self.attrs(&parent).await.ok();
            encode_remove_ok(w, parent_attrs.as_ref());
            Ok(())
        }
    }

    fn rename(
        &self,
        from_dir: &NfsFh3,
        from_name: &str,
        to_dir: &NfsFh3,
        to_name: &str,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            validate_name(from_name)?;
            validate_name(to_name)?;
            let from_parent = self.resolve(from_dir)?;
            let to_parent = self.resolve(to_dir)?;
            let source_context = self.directory_context(&from_parent).await?;
            let destination_context = self.directory_context(&to_parent).await?;
            if source_context.device_key != destination_context.device_key {
                return Err(Nfsstat3::Xdev);
            }
            if !source_context.writable || !destination_context.writable {
                return Err(Nfsstat3::Rofs);
            }
            let source = self.lookup_in_directory(&source_context, from_name).await?;
            let staged_id = self.staged_id(&source).await;
            if let Ok(existing) = self
                .lookup_in_directory(&destination_context, to_name)
                .await
            {
                if existing != source {
                    self.remove_handle(existing).await?;
                }
            }
            self.rename_handle(
                source,
                &source_context,
                &destination_context,
                from_name,
                to_name,
            )
            .await?;
            if let Some(id) = staged_id {
                self.schedule_commit(id).await?;
            }
            let from_attrs = self.attrs(&from_parent).await.ok();
            let to_attrs = self.attrs(&to_parent).await.ok();
            encode_rename_ok(w, from_attrs.as_ref(), to_attrs.as_ref());
            Ok(())
        }
    }

    fn readdir(
        &self,
        dir_fh: &NfsFh3,
        cookie: u64,
        cookieverf: [u8; 8],
        count: u32,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            self.validate_cookie(cookie, cookieverf)?;
            let dir = self.resolve(dir_fh)?;
            let all = self.directory_entries(&dir).await?;
            let start = usize::try_from(cookie).unwrap_or(usize::MAX).min(all.len());
            let mut entries = Vec::new();
            let mut used = 128_usize;
            for (index, (name, handle)) in all.iter().enumerate().skip(start) {
                let estimate = 32 + name.len().next_multiple_of(4);
                if !entries.is_empty() && used.saturating_add(estimate) > count as usize {
                    break;
                }
                if entries.is_empty() && used.saturating_add(estimate) > count as usize {
                    return Err(Nfsstat3::TooSmall);
                }
                used += estimate;
                entries.push(Entry3 {
                    fileid: self.handles.intern(handle.clone()),
                    name: name.clone(),
                    cookie: u64::try_from(index + 1).unwrap_or(u64::MAX),
                });
            }
            let eof = start.saturating_add(entries.len()) >= all.len();
            let attrs = self.attrs(&dir).await.ok();
            encode_readdir_ok(w, attrs.as_ref(), &self.cookie_verifier, &entries, eof);
            Ok(())
        }
    }

    fn readdirplus(
        &self,
        dir_fh: &NfsFh3,
        cookie: u64,
        cookieverf: [u8; 8],
        _dircount: u32,
        maxcount: u32,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            self.validate_cookie(cookie, cookieverf)?;
            let dir = self.resolve(dir_fh)?;
            let all = self.directory_entries(&dir).await?;
            let start = usize::try_from(cookie).unwrap_or(usize::MAX).min(all.len());
            let mut entries = Vec::new();
            let mut used = 160_usize;
            for (index, (name, handle)) in all.iter().enumerate().skip(start) {
                let estimate = 176 + name.len().next_multiple_of(4);
                if !entries.is_empty() && used.saturating_add(estimate) > maxcount as usize {
                    break;
                }
                if entries.is_empty() && used.saturating_add(estimate) > maxcount as usize {
                    return Err(Nfsstat3::TooSmall);
                }
                used += estimate;
                let inode = self.handles.intern(handle.clone());
                entries.push(Entryplus3 {
                    fileid: inode,
                    name: name.clone(),
                    cookie: u64::try_from(index + 1).unwrap_or(u64::MAX),
                    attr: self.attrs(handle).await.ok(),
                    fh: Some(NfsFh3::new(inode, NFS_FSID)),
                });
            }
            let eof = start.saturating_add(entries.len()) >= all.len();
            let attrs = self.attrs(&dir).await.ok();
            encode_readdirplus_ok(w, attrs.as_ref(), &self.cookie_verifier, &entries, eof);
            Ok(())
        }
    }

    fn fsstat(&self, fh: &NfsFh3, w: &mut XdrWriter) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let attrs = self.attrs(&handle).await?;
            let devices = self.manager.summaries().await.map_err(map_error)?;
            let reported_total: u64 = devices
                .iter()
                .flat_map(|device| &device.storages)
                .map(|storage| storage.total_bytes)
                .sum();
            let reported_free: u64 = devices
                .iter()
                .flat_map(|device| &device.storages)
                .map(|storage| storage.free_bytes)
                .sum();
            let total = reported_total.max(FALLBACK_CAPACITY);
            let free = if reported_total == 0 {
                FALLBACK_CAPACITY
            } else {
                reported_free.min(total)
            };
            encode_fsstat_ok(w, &attrs, total, free, free, 1 << 32, 1 << 31, 1 << 31, 1);
            Ok(())
        }
    }

    fn fsinfo(&self, fh: &NfsFh3, w: &mut XdrWriter) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let attrs = self.attrs(&handle).await?;
            encode_fsinfo_ok(
                w,
                &attrs,
                MAX_IO_SIZE,
                PREFERRED_IO_SIZE,
                4096,
                MAX_IO_SIZE,
                PREFERRED_IO_SIZE,
                4096,
                PREFERRED_IO_SIZE,
                i64::MAX as u64,
                FSF3_HOMOGENEOUS,
            );
            Ok(())
        }
    }

    fn pathconf(&self, fh: &NfsFh3, w: &mut XdrWriter) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            let attrs = self.attrs(&handle).await?;
            encode_pathconf_ok(w, &attrs, 1, 254);
            Ok(())
        }
    }

    fn commit(
        &self,
        fh: &NfsFh3,
        _offset: u64,
        _count: u32,
        w: &mut XdrWriter,
    ) -> impl Future<Output = NfsResult> {
        async move {
            let handle = self.resolve(fh)?;
            self.commit_handle(&handle).await?;
            let attrs = self.attrs(&handle).await?;
            encode_commit_ok(w, &attrs, &self.write_verifier);
            Ok(())
        }
    }
}

type NfsFsResult<T> = std::result::Result<T, Nfsstat3>;

fn directory_attrs(fileid: u64, uid: u32, gid: u32, timestamp: Nfstime3) -> Fattr3 {
    Fattr3 {
        ftype: Ftype3::Dir,
        mode: 0o755,
        nlink: 2,
        uid,
        gid,
        size: 0,
        used: 0,
        rdev: Specdata3::default(),
        fsid: NFS_FSID,
        fileid,
        atime: timestamp,
        mtime: timestamp,
        ctime: timestamp,
    }
}

fn object_attrs(fileid: u64, object: &ObjectEntry, uid: u32, gid: u32) -> Fattr3 {
    let created = object
        .created
        .and_then(timestamp_from_mtp)
        .unwrap_or_default();
    let modified = object
        .modified
        .and_then(timestamp_from_mtp)
        .unwrap_or(created);
    Fattr3 {
        ftype: if object.is_dir {
            Ftype3::Dir
        } else {
            Ftype3::Reg
        },
        mode: if object.is_dir { 0o755 } else { 0o644 },
        nlink: if object.is_dir { 2 } else { 1 },
        uid,
        gid,
        size: object.size,
        used: object.size,
        rdev: Specdata3::default(),
        fsid: NFS_FSID,
        fileid,
        atime: modified,
        mtime: modified,
        ctime: created,
    }
}

fn timestamp_from_mtp(value: mtp_rs::mtp::DateTime) -> Option<Nfstime3> {
    let month = Month::try_from(value.month).ok()?;
    let date = Date::from_calendar_date(i32::from(value.year), month, value.day).ok()?;
    let time = Time::from_hms(value.hour, value.minute, value.second).ok()?;
    let seconds = PrimitiveDateTime::new(date, time)
        .assume_utc()
        .unix_timestamp();
    Some(Nfstime3::from_secs(u64::try_from(seconds).unwrap_or(0)))
}

fn timestamp_from_system(value: SystemTime) -> Nfstime3 {
    let duration = value.duration_since(UNIX_EPOCH).unwrap_or_default();
    Nfstime3::new(
        u32::try_from(duration.as_secs()).unwrap_or(u32::MAX),
        duration.subsec_nanos(),
    )
}

fn ensure_generation(summary: &DeviceSummary, generation: u64) -> NfsFsResult<()> {
    if summary.generation == generation {
        Ok(())
    } else {
        Err(Nfsstat3::Stale)
    }
}

fn validate_name(name: &str) -> NfsFsResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(Nfsstat3::Inval);
    }
    if name.len() > 254 {
        return Err(Nfsstat3::Nametoolong);
    }
    Ok(())
}

fn safe_component(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| match character {
            '/' | ':' | '\0' => '_',
            _ => character,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "MTP device".to_owned()
    } else {
        cleaned.to_owned()
    }
}

fn serial_suffix(serial: &str) -> String {
    let chars: Vec<char> = serial.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

fn map_error(error: Error) -> Nfsstat3 {
    match error {
        Error::Disconnected => Nfsstat3::Stale,
        Error::NotFound => Nfsstat3::Noent,
        Error::Unsupported(_) => Nfsstat3::NotSupp,
        Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound => Nfsstat3::Noent,
        Error::Io(io) if io.kind() == std::io::ErrorKind::StorageFull => Nfsstat3::Nospc,
        Error::Operation(_) | Error::Mtp(_) | Error::Io(_) => Nfsstat3::Io,
        Error::DaemonUnavailable | Error::InvalidResponse(_) | Error::Json(_) => {
            Nfsstat3::ServerFault
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_and_suffixes_are_short() {
        assert_eq!(safe_component("Pixel/Pro"), "Pixel_Pro");
        assert_eq!(serial_suffix("ABCDEF12"), "EF12");
    }

    #[test]
    fn handles_are_stable_and_reject_other_filesystems() {
        let table = HandleTable::new();
        let handle = NodeHandle::Storage {
            device_key: "phone".into(),
            generation: 1,
            storage_id: 2,
        };
        let inode = table.intern(handle.clone());
        assert_eq!(inode, table.intern(handle.clone()));
        assert_eq!(table.resolve(&NfsFh3::new(inode, NFS_FSID)), Ok(handle));
        assert_eq!(
            table.resolve(&NfsFh3::new(inode, 99)),
            Err(Nfsstat3::Badhandle)
        );
    }
}
