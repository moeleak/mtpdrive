//! TCP server for NFSv3 with per-CPU compio runtimes and SO_REUSEPORT.
//! Each thread runs an independent single-threaded compio runtime so
//! per-connection tasks don't need `Send` futures -- the `Nfs3Filesystem`
//! trait doesn't bound its returned futures with `Send`.
use std::sync::Arc;
use std::sync::mpsc;

use bytes::BytesMut;
use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncWrite};
use compio_net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::Nfs3Filesystem;
use crate::config::{NfsServerConfig, bind_reuseport_std};
use crate::dispatch;
use crate::nfs3_types::NfsFh3;
use crate::rpc::{self, RpcCallHeader, frame_reply};
use crate::xdr::XdrReader;

/// Run the NFS server on per-CPU compio runtimes. Blocks until shutdown.
///
/// Setup proceeds in two phases so failures are deterministic:
///
/// 1. The main thread binds every listening socket up front. A bind
///    failure returns immediately without spawning any workers.
/// 2. Workers are spawned, each given an already-bound listener. The
///    only setup that can still fail inside a worker is constructing
///    its compio runtime; workers report that via a setup channel and
///    the main thread signals shutdown to siblings if any worker fails.
///
/// During shutdown the workers wrap their accept call in
/// `CancellationToken::run_until_cancelled`, so cancelling the token
/// wakes every worker immediately and `run()` joins every spawned
/// worker deterministically.
pub fn run<F: Nfs3Filesystem>(fs: F, config: NfsServerConfig) -> std::io::Result<()> {
    config.validate()?;
    let fs = Arc::new(fs);
    let root_fh = NfsFh3::new(1, config.fsid); // inode 1 = root
    let addr: std::net::SocketAddr = ([0, 0, 0, 0], config.port).into();
    let num_threads = config.num_threads;
    let port = config.port;
    let max_rpc_fragment_bytes = config.max_rpc_fragment_bytes;

    // Phase 1: bind every listener on the main thread.
    let mut std_listeners = Vec::with_capacity(num_threads);
    for _ in 0..num_threads {
        std_listeners.push(bind_reuseport_std(addr)?);
    }

    let shutdown = CancellationToken::new();
    let (setup_tx, setup_rx) = mpsc::sync_channel::<std::io::Result<()>>(num_threads);
    let mut handles = Vec::with_capacity(num_threads);
    let mut spawn_err: Option<std::io::Error> = None;

    // Phase 2: spawn workers, each owning a pre-bound listener.
    for (thread_id, std_listener) in std_listeners.into_iter().enumerate() {
        let fs = fs.clone();
        let root_fh = root_fh.clone();
        let setup_tx = setup_tx.clone();
        let shutdown_worker = shutdown.clone();

        let result = std::thread::Builder::new()
            .name(format!("nfs-{thread_id}"))
            .spawn(move || {
                worker_main(
                    thread_id,
                    port,
                    std_listener,
                    fs,
                    root_fh,
                    shutdown_worker,
                    setup_tx,
                    max_rpc_fragment_bytes,
                );
            });

        match result {
            Ok(h) => handles.push(h),
            Err(e) => {
                spawn_err = Some(e);
                break;
            }
        }
    }

    drop(setup_tx);

    // Drain setup results from every successfully-spawned worker.
    let mut setup_err: Option<std::io::Error> = None;
    for _ in 0..handles.len() {
        match setup_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                setup_err.get_or_insert(e);
            }
            Err(_) => {
                setup_err.get_or_insert_with(|| {
                    std::io::Error::other("NFS worker thread died before reporting setup result")
                });
            }
        }
    }

    if let Some(e) = spawn_err.or(setup_err) {
        // Cancel wakes every worker's run_until_cancelled accept call.
        shutdown.cancel();
        for h in handles {
            let _ = h.join();
        }
        return Err(e);
    }

    for h in handles {
        if let Err(panic) = h.join() {
            std::panic::resume_unwind(panic);
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn worker_main<F: Nfs3Filesystem>(
    thread_id: usize,
    port: u16,
    std_listener: std::net::TcpListener,
    fs: Arc<F>,
    root_fh: NfsFh3,
    shutdown: CancellationToken,
    setup_tx: mpsc::SyncSender<std::io::Result<()>>,
    max_rpc_fragment_bytes: usize,
) {
    let rt = match compio_runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            let _ = setup_tx.send(Err(e));
            return;
        }
    };

    rt.block_on(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(e) => {
                let _ = setup_tx.send(Err(e));
                return;
            }
        };
        tracing::info!(thread = thread_id, port, "NFS listener started");
        let _ = setup_tx.send(Ok(()));
        drop(setup_tx);

        loop {
            // run_until_cancelled returns None when the token is cancelled,
            // wrapping the accept result otherwise. No polling required.
            match shutdown.run_until_cancelled(listener.accept()).await {
                Some(Ok((stream, peer))) => {
                    tracing::debug!(%peer, "NFS connection accepted");
                    let fs = fs.clone();
                    let root_fh = root_fh.clone();
                    compio_runtime::spawn(async move {
                        if let Err(e) =
                            handle_connection(stream, &fs, &root_fh, max_rpc_fragment_bytes).await
                        {
                            tracing::debug!(%peer, error = %e, "NFS connection closed");
                        }
                    })
                    .detach();
                }
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "NFS accept error");
                }
                None => break,
            }
        }
    });
}

/// Handle a single NFS TCP connection.
async fn handle_connection<F: Nfs3Filesystem>(
    mut stream: compio_net::TcpStream,
    fs: &Arc<F>,
    root_fh: &NfsFh3,
    max_rpc_fragment_bytes: usize,
) -> std::io::Result<()> {
    let mut recv_buf = BytesMut::with_capacity(64 * 1024);
    let mut message = BytesMut::new();

    loop {
        // Read fragment header (4 bytes).
        while recv_buf.len() < 4 {
            let n = read_some(&mut stream, &mut recv_buf).await?;
            if n == 0 {
                if message.is_empty() {
                    return Ok(());
                }
                return Err(std::io::Error::other("connection closed mid-message"));
            }
        }

        let (frag_len, last) = rpc::read_record_mark(&recv_buf)
            .ok_or_else(|| std::io::Error::other("invalid record mark"))?;

        if (frag_len as usize) > max_rpc_fragment_bytes {
            return Err(std::io::Error::other(format!(
                "rpc fragment of {frag_len} bytes exceeds limit of {max_rpc_fragment_bytes}"
            )));
        }
        let total_needed = 4 + frag_len as usize;

        while recv_buf.len() < total_needed {
            let n = read_some(&mut stream, &mut recv_buf).await?;
            if n == 0 {
                return Err(std::io::Error::other("connection closed mid-fragment"));
            }
        }

        let frag = recv_buf.split_to(total_needed);
        message.extend_from_slice(&frag[4..]);

        if message.len() > max_rpc_fragment_bytes {
            return Err(std::io::Error::other(format!(
                "reassembled rpc message of {} bytes exceeds limit of {}",
                message.len(),
                max_rpc_fragment_bytes
            )));
        }

        if !last {
            continue;
        }

        // Last fragment received: parse and dispatch the complete message.
        let header_result = {
            let mut reader = XdrReader::new(&message);
            RpcCallHeader::decode(&mut reader).map(|h| (h, reader.position()))
        };
        let (header, header_end) = match header_result {
            Ok(v) => v,
            Err(_) => {
                tracing::debug!("Failed to decode RPC header, skipping");
                message.clear();
                continue;
            }
        };
        let args_buf = &message[header_end..];

        let reply_body =
            dispatch::dispatch_rpc(fs, &header, args_buf, root_fh, max_rpc_fragment_bytes).await;
        let reply_frame = frame_reply(&reply_body);

        write_all(&mut stream, &reply_frame).await?;
        message.clear();
    }
}

async fn read_some(
    stream: &mut compio_net::TcpStream,
    buf: &mut BytesMut,
) -> std::io::Result<usize> {
    let tmp = vec![0u8; 32 * 1024];
    let BufResult(result, tmp) = stream.read(tmp).await;
    let n = result?;
    buf.extend_from_slice(&tmp[..n]);
    Ok(n)
}

async fn write_all(stream: &mut compio_net::TcpStream, data: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let chunk = data[offset..].to_vec();
        let BufResult(result, _) = stream.write(chunk).await;
        let n = result?;
        if n == 0 {
            return Err(std::io::Error::other("write returned 0"));
        }
        offset += n;
    }
    Ok(())
}
