#![doc = include_str!("../README.md")]

#[cfg(not(any(feature = "tokio-runtime", feature = "compio-runtime")))]
compile_error!(
    "fractal-nfs requires at least one runtime feature: \
     `tokio-runtime` (default) or `compio-runtime`"
);

pub mod config;
pub mod dispatch;
pub mod mount;
pub mod nfs3_types;
pub mod nfs3_wire;
pub mod rpc;
pub mod xdr;

#[cfg(feature = "compio-runtime")]
pub mod compio_server;
#[cfg(feature = "tokio-runtime")]
pub mod tokio_server;

pub use config::NfsServerConfig;
pub use nfs3_types::*;

// Re-export `run` so callers don't have to pick a backend by name. Tokio
// is preferred when both features are enabled.
#[cfg(all(feature = "compio-runtime", not(feature = "tokio-runtime")))]
pub use compio_server::run;
#[cfg(feature = "tokio-runtime")]
pub use tokio_server::{run, run_until};

/// Result type for NFS operations: Ok(()) means the success response was
/// already encoded into the XdrWriter; Err(status) is encoded by the
/// dispatch layer so filesystem code only needs to return the error code.
pub type NfsResult = Result<(), Nfsstat3>;

/// Trait for implementing an NFSv3 filesystem.
///
/// Each method receives pre-decoded arguments and an XdrWriter for encoding
/// the response body (status + result data). The RPC accepted header is
/// already written before these methods are called.
///
/// On success, methods encode the OK response into the writer and return
/// `Ok(())`. On failure, methods return `Err(Nfsstat3::...)` and the
/// dispatch layer encodes the error response (truncating any partial writes).
///
/// Returned futures are not required to be `Send`: each runtime backend
/// (tokio or compio) drives them on a single-threaded runtime per CPU.
/// The trait itself is `Send + Sync` so the filesystem can be shared
/// across those per-CPU threads via `Arc`.
pub trait Nfs3Filesystem: Send + Sync + 'static {
    fn getattr(
        &self,
        fh: &NfsFh3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn setattr(
        &self,
        fh: &NfsFh3,
        attrs: &Sattr3,
        guard_ctime: Option<Nfstime3>,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, attrs, guard_ctime, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn lookup(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, name, w);
        async { Err(Nfsstat3::Noent) }
    }

    fn access(
        &self,
        fh: &NfsFh3,
        access: u32,
        uid: u32,
        gid: u32,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, access, uid, gid, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn readlink(
        &self,
        fh: &NfsFh3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, w);
        async { Err(Nfsstat3::Inval) }
    }

    fn read(
        &self,
        fh: &NfsFh3,
        offset: u64,
        count: u32,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, offset, count, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn write(
        &self,
        fh: &NfsFh3,
        offset: u64,
        data: &[u8],
        stable: StableHow,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, offset, data, stable, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn create(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        how: &CreateHow3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, name, how, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn mkdir(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        attrs: &Sattr3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, name, attrs, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn remove(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, name, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn rmdir(
        &self,
        dir_fh: &NfsFh3,
        name: &str,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, name, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn rename(
        &self,
        from_dir: &NfsFh3,
        from_name: &str,
        to_dir: &NfsFh3,
        to_name: &str,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (from_dir, from_name, to_dir, to_name, w);
        async { Err(Nfsstat3::Rofs) }
    }

    fn readdir(
        &self,
        dir_fh: &NfsFh3,
        cookie: u64,
        cookieverf: [u8; 8],
        count: u32,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, cookie, cookieverf, count, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn readdirplus(
        &self,
        dir_fh: &NfsFh3,
        cookie: u64,
        cookieverf: [u8; 8],
        dircount: u32,
        maxcount: u32,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (dir_fh, cookie, cookieverf, dircount, maxcount, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn fsstat(
        &self,
        fh: &NfsFh3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn fsinfo(
        &self,
        fh: &NfsFh3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn pathconf(
        &self,
        fh: &NfsFh3,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, w);
        async { Err(Nfsstat3::NotSupp) }
    }

    fn commit(
        &self,
        fh: &NfsFh3,
        offset: u64,
        count: u32,
        w: &mut xdr::XdrWriter,
    ) -> impl std::future::Future<Output = NfsResult> {
        let _ = (fh, offset, count, w);
        async { Err(Nfsstat3::NotSupp) }
    }
}
