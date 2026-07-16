/// RPC dispatch: route incoming RPC calls to Nfs3Filesystem trait methods.
use std::sync::Arc;

use crate::Nfs3Filesystem;
use crate::mount;
use crate::nfs3_types::*;
use crate::nfs3_wire::{self, *};
use crate::rpc::{self, RpcCallHeader};
use crate::xdr::{XdrReader, XdrWriter};

pub const NFS_PROGRAM: u32 = 100003;
pub const NFS_VERSION: u32 = 3;

/// Dispatch a single RPC call. Returns the reply as an XdrWriter.
///
/// `max_reply_bytes` bounds the per-call reply size we'll preallocate
/// (currently only consulted by NFSPROC3_READ to clamp `args.count`).
pub async fn dispatch_rpc<F: Nfs3Filesystem>(
    fs: &Arc<F>,
    header: &RpcCallHeader,
    args_buf: &[u8],
    root_fh: &NfsFh3,
    max_reply_bytes: usize,
) -> XdrWriter {
    // Check RPC version
    if header.rpc_version != rpc::RPC_VERSION {
        let mut w = XdrWriter::new();
        rpc::write_reply_prog_unavail(&mut w, header.xid);
        return w;
    }

    // Route by program
    match header.program {
        mount::MOUNT_PROGRAM => {
            mount::handle_mount_call(header.xid, header.procedure, args_buf, root_fh)
        }
        NFS_PROGRAM => {
            if header.prog_version != NFS_VERSION {
                let mut w = XdrWriter::new();
                rpc::write_reply_prog_unavail(&mut w, header.xid);
                return w;
            }
            dispatch_nfs3(fs, header, args_buf, max_reply_bytes).await
        }
        _ => {
            let mut w = XdrWriter::new();
            rpc::write_reply_prog_unavail(&mut w, header.xid);
            w
        }
    }
}

async fn dispatch_nfs3<F: Nfs3Filesystem>(
    fs: &Arc<F>,
    header: &RpcCallHeader,
    args_buf: &[u8],
    max_reply_bytes: usize,
) -> XdrWriter {
    let mut r = XdrReader::new(args_buf);
    let xid = header.xid;
    let uid = header.cred_uid;
    let gid = header.cred_gid;

    match header.procedure {
        NFSPROC3_NULL => {
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            w
        }

        NFSPROC3_GETATTR => {
            let args = match GetattrArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.getattr(&args.fh, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_getattr_err(&mut w, status);
            }
            w
        }

        NFSPROC3_SETATTR => {
            let args = match SetattrArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .setattr(&args.fh, &args.new_attrs, args.guard_ctime, &mut w)
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_setattr_err(&mut w, status);
            }
            w
        }

        NFSPROC3_LOOKUP => {
            let args = match LookupArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.lookup(&args.dir_fh, &args.name, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_lookup_err(&mut w, status, None);
            }
            w
        }

        NFSPROC3_ACCESS => {
            let args = match AccessArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.access(&args.fh, args.access, uid, gid, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_access_err(&mut w, status);
            }
            w
        }

        NFSPROC3_READLINK => {
            let args = match ReadlinkArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.readlink(&args.fh, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_readlink_err(&mut w, status);
            }
            w
        }

        NFSPROC3_READ => {
            let args = match ReadArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            // Clamp count so a malicious or buggy client can't make us
            // allocate gigabytes per request. The cap matches the
            // server's max_rpc_fragment_bytes (default 16 MiB).
            let count = std::cmp::min(args.count as usize, max_reply_bytes) as u32;
            let mut w = XdrWriter::with_capacity(count as usize + 256);
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.read(&args.fh, args.offset, count, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_read_err(&mut w, status);
            }
            w
        }

        NFSPROC3_WRITE => {
            let args = match WriteArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .write(&args.fh, args.offset, &args.data, args.stable, &mut w)
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_write_err(&mut w, status);
            }
            w
        }

        NFSPROC3_CREATE => {
            let args = match CreateArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.create(&args.dir_fh, &args.name, &args.how, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_create_err(&mut w, status);
            }
            w
        }

        NFSPROC3_MKDIR => {
            let args = match MkdirArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .mkdir(&args.dir_fh, &args.name, &args.attrs, &mut w)
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_mkdir_err(&mut w, status);
            }
            w
        }

        NFSPROC3_REMOVE => {
            let args = match RemoveArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.remove(&args.dir_fh, &args.name, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_remove_err(&mut w, status);
            }
            w
        }

        NFSPROC3_RMDIR => {
            let args = match RemoveArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.rmdir(&args.dir_fh, &args.name, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_remove_err(&mut w, status);
            }
            w
        }

        NFSPROC3_RENAME => {
            let args = match RenameArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .rename(
                    &args.from_dir,
                    &args.from_name,
                    &args.to_dir,
                    &args.to_name,
                    &mut w,
                )
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_rename_err(&mut w, status);
            }
            w
        }

        NFSPROC3_READDIR => {
            let args = match ReaddirArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .readdir(
                    &args.dir_fh,
                    args.cookie,
                    args.cookieverf,
                    args.count,
                    &mut w,
                )
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_readdir_err(&mut w, status);
            }
            w
        }

        NFSPROC3_READDIRPLUS => {
            let args = match ReaddirplusArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs
                .readdirplus(
                    &args.dir_fh,
                    args.cookie,
                    args.cookieverf,
                    args.dircount,
                    args.maxcount,
                    &mut w,
                )
                .await
            {
                w.truncate(pos);
                nfs3_wire::encode_readdirplus_err(&mut w, status);
            }
            w
        }

        NFSPROC3_FSSTAT => {
            let args = match GetattrArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.fsstat(&args.fh, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_fsstat_err(&mut w, status);
            }
            w
        }

        NFSPROC3_FSINFO => {
            let args = match GetattrArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.fsinfo(&args.fh, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_fsinfo_err(&mut w, status);
            }
            w
        }

        NFSPROC3_PATHCONF => {
            let args = match GetattrArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.pathconf(&args.fh, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_pathconf_err(&mut w, status);
            }
            w
        }

        NFSPROC3_COMMIT => {
            let args = match CommitArgs::decode(&mut r) {
                Ok(a) => a,
                Err(_) => return garbage_args(xid),
            };
            let mut w = XdrWriter::new();
            rpc::write_reply_accepted(&mut w, xid);
            let pos = w.len();
            if let Err(status) = fs.commit(&args.fh, args.offset, args.count, &mut w).await {
                w.truncate(pos);
                nfs3_wire::encode_commit_err(&mut w, status);
            }
            w
        }

        _ => {
            let mut w = XdrWriter::new();
            rpc::write_reply_proc_unavail(&mut w, xid);
            w
        }
    }
}

fn garbage_args(xid: u32) -> XdrWriter {
    let mut w = XdrWriter::new();
    rpc::write_reply_garbage_args(&mut w, xid);
    w
}
