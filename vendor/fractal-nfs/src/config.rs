//! Runtime-agnostic server configuration.

/// Default upper bound on the size of a single RPC record fragment we'll
/// accept from a client (16 MiB). Caps both the buffer we'll grow to
/// reassemble a request and the size of a single READ reply. Generous
/// relative to typical NFS rsize/wsize (1 MiB), tight enough to prevent
/// a hostile client from forcing multi-GB allocations.
pub const DEFAULT_MAX_RPC_FRAGMENT_BYTES: usize = 16 * 1024 * 1024;

/// Configuration for the NFS server, shared by both the compio and tokio
/// backends.
pub struct NfsServerConfig {
    /// NFS + MOUNT listen port (both on same port for simplicity).
    pub port: u16,
    /// Number of per-CPU listener threads. Each thread runs an independent
    /// runtime and binds to the port with SO_REUSEPORT, so the kernel
    /// load-balances incoming connections across them.
    pub num_threads: usize,
    /// FSID for file handles.
    pub fsid: u64,
    /// Upper bound on a single RPC record fragment we'll accept from a
    /// client. Defaults to `DEFAULT_MAX_RPC_FRAGMENT_BYTES`. Connections
    /// announcing a larger record are dropped; READ replies are clamped
    /// to this size.
    pub max_rpc_fragment_bytes: usize,
}

impl Default for NfsServerConfig {
    fn default() -> Self {
        Self {
            port: 2049,
            num_threads: num_cpus(),
            fsid: 1,
            max_rpc_fragment_bytes: DEFAULT_MAX_RPC_FRAGMENT_BYTES,
        }
    }
}

impl NfsServerConfig {
    /// Validate fields whose obviously-broken values would otherwise let
    /// `run()` exit `Ok(())` without binding anything (e.g. `num_threads:
    /// 0`). Backends call this before phase 1 so misconfiguration is
    /// rejected loudly.
    pub fn validate(&self) -> std::io::Result<()> {
        if self.num_threads == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NfsServerConfig::num_threads must be at least 1",
            ));
        }
        Ok(())
    }
}

pub(crate) fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Build a non-blocking, SO_REUSEPORT-enabled, listening `std::net::TcpListener`
/// for the given address. Each runtime backend wraps it into its own
/// listener type.
pub(crate) fn bind_reuseport_std(
    addr: std::net::SocketAddr,
) -> std::io::Result<std::net::TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_port(true)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}
