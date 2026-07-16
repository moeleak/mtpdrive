# fractal-nfs

[![CI](https://github.com/fractalbits-labs/fractalbits-main/actions/workflows/ci.yml/badge.svg)](https://github.com/fractalbits-labs/fractalbits-main/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/fractal-nfs.svg)](https://crates.io/crates/fractal-nfs)
[![docs.rs](https://docs.rs/fractal-nfs/badge.svg)](https://docs.rs/fractal-nfs)
[![source](https://img.shields.io/badge/source-GitHub-blue)](https://github.com/fractalbits-labs/fractalbits-main/tree/main/crates/fs_server/fractal-nfs)

A minimal, embeddable **NFSv3** server in Rust. Implement one trait
(`Nfs3Filesystem`), pick an async runtime (tokio by default, compio
optional), call `run(...)`, and your filesystem is reachable over NFS.

The crate handles ONC RPC framing, XDR encoding/decoding, the MOUNT
protocol, and NFSv3 procedure dispatch. Filesystem code only deals with
pre-decoded arguments and writes typed responses through helpers.

## Features

- **Runtime-agnostic** -- pick `tokio-runtime` (default) or
  `compio-runtime` via cargo features; both can be enabled at once for
  multi-target consumers
- **Per-CPU listener threads** with `SO_REUSEPORT` so the kernel
  load-balances connections; each thread runs an independent
  single-threaded runtime so `Nfs3Filesystem` futures don't need `Send`
- **Single-trait surface** -- implement `Nfs3Filesystem` (async methods,
  most with sensible defaults like `Rofs` or `NotSupp`) and you're done
- **Read-only and read-write** -- defaults make read-only filesystems a
  one-liner; override `write`/`create`/`mkdir`/etc. when you need them
- **Symlink support** -- includes `readlink` for filesystems that
  surface symbolic links
- **MOUNT v3 included** -- single TCP port for both NFS and MOUNT so no
  separate mountd process is needed
- **No portmapper required** -- clients mount with explicit
  `port=N,mountport=N` options

## Requirements

- **Rust edition 2024** (Rust 1.85+)
- An NFSv3 client (Linux `mount -t nfs`, macOS, etc.) -- the server is
  cross-platform but only Linux/macOS are exercised in CI

## Usage

Implement `Nfs3Filesystem` and call `run`:

```rust,no_run
use fractal_nfs::{Nfs3Filesystem, NfsServerConfig};

struct MyFs;

impl Nfs3Filesystem for MyFs {
    // Override only the operations your filesystem implements.
    // All methods default to a sensible NFS error (NotSupp, Noent, Rofs, ...).
}

fn main() -> std::io::Result<()> {
    let cfg = NfsServerConfig {
        port: 2049,
        num_threads: 4,
        ..Default::default()
    };
    fractal_nfs::run(MyFs, cfg)
}
```

Then mount from a client:

```sh
sudo mount -o port=2049,mountport=2049,vers=3,tcp,nolock -t nfs localhost:/ /mnt/myfs
```

## Architecture

```text
NfsServerConfig { port, num_threads, fsid }
        |
        +-- bind_reuseport_std(addr)  (SO_REUSEPORT listening socket)
        +-- N x std::thread::spawn
              |
              +-- runtime::block_on(...)
                    |
                    +-- listener.accept().await
                          |
                          +-- spawn(handle_connection(stream, &fs, &root_fh))
                                |
                                +-- read TCP record mark (4 bytes)
                                +-- read fragment payload
                                +-- decode RPC call header
                                +-- dispatch::dispatch_rpc(fs, header, args, root_fh)
                                |     |
                                |     +-- MOUNT program -> mount::handle_mount_call
                                |     +-- NFS program -> NFSPROC3_* -> trait method
                                +-- frame_reply(reply_body) (prepend record mark)
                                +-- stream.write_all(reply_frame)
```

Each listener thread owns its own single-threaded runtime (`compio::Runtime`
or `tokio::current_thread + LocalSet`). All threads share an `Arc<F>` so
the filesystem implementation must be `Send + Sync + 'static`, but
returned futures don't need `Send` -- handy when the filesystem holds
non-`Send` handles.

## Nfs3Filesystem Trait

Implement
[`Nfs3Filesystem`](https://docs.rs/fractal-nfs/latest/fractal_nfs/trait.Nfs3Filesystem.html)
for your filesystem. Every method has a default that returns an NFS
error, so an empty `impl Nfs3Filesystem for MyFs {}` is legal -- you
override only what your filesystem actually supports.

| Operation | Default | Description |
|-----------|---------|-------------|
| `getattr` | `NotSupp` | Get file attributes |
| `setattr` | `NotSupp` | Set file attributes |
| `lookup` | `Noent` | Look up a directory entry by name |
| `access` | `NotSupp` | Check access rights |
| `readlink` | `Inval` | Read a symbolic link target |
| `read` | `NotSupp` | Read file data |
| `write` | `Rofs` | Write file data |
| `create` | `Rofs` | Create a regular file (carries `CreateHow3`: attrs or exclusive verifier) |
| `mkdir` | `Rofs` | Create a directory |
| `remove` | `Rofs` | Unlink a file |
| `rmdir` | `Rofs` | Remove an empty directory |
| `rename` | `Rofs` | Rename a file or directory |
| `readdir` | `NotSupp` | Read directory entries |
| `readdirplus` | `NotSupp` | Read directory entries with attributes |
| `fsstat` | `NotSupp` | Filesystem space + inode statistics |
| `fsinfo` | `NotSupp` | Static filesystem info (block sizes, limits) |
| `pathconf` | `NotSupp` | POSIX `pathconf`-style limits |
| `commit` | `NotSupp` | Commit unstable writes (no-op when writes are FILE_SYNC) |

A practically useful filesystem will at minimum override `getattr`,
`lookup`, `access`, `fsinfo`, and `pathconf` -- without those, the
kernel client will fail to mount or fail every operation. Add `read` +
`readdir` (+ `readdirplus`) for a read-only file tree, then any
write-side ops you need.

Each method receives an `XdrWriter` and, on success, encodes the OK
response using helpers from
[`nfs3_wire`](https://docs.rs/fractal-nfs/latest/fractal_nfs/nfs3_wire/index.html)
(e.g. `encode_getattr_ok`, `encode_lookup_ok`). On error, return
`Err(Nfsstat3::...)` and the dispatch layer encodes the failure response
for you.

## Features

| Feature | Default | Pulls in | Notes |
|---------|---------|----------|-------|
| `tokio-runtime` | yes | `tokio` (`rt`, `net`, `io-util`) | Per-CPU `current_thread` runtimes + `LocalSet` |
| `compio-runtime` | no | `compio-runtime`, `compio-net`, `compio-io`, `compio-buf` | Per-CPU compio runtimes; integrates with crates already using compio |

## License

Licensed under [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
