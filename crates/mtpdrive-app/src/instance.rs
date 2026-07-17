use fs2::FileExt;
use mtpdrive_core::AppPaths;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

const UI_SHOW_SIGNAL: &[u8] = b"show";
const SIGNAL_ATTEMPTS: usize = 10;
const SIGNAL_RETRY_DELAY: Duration = Duration::from_millis(25);
static UI_INSTANCE: OnceLock<UnixDatagram> = OnceLock::new();
static UI_LOCK: OnceLock<File> = OnceLock::new();

pub(crate) fn acquire() -> bool {
    let Ok(paths) = AppPaths::discover() else {
        return true;
    };
    if paths.ensure().is_err() {
        return true;
    }
    let socket_path = paths.support_dir.join("ui.sock");
    let lock_path = paths.support_dir.join("ui.lock");
    match acquire_lock(&lock_path) {
        Ok(Some(lock)) => {
            if UI_LOCK.set(lock).is_err() {
                return false;
            }
            let _ = std::fs::remove_file(&socket_path);
            UnixDatagram::bind(&socket_path).is_ok_and(install)
        }
        Ok(None) => {
            signal_existing(&socket_path);
            false
        }
        Err(_) => true,
    }
}

fn acquire_lock(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error),
    }
}

fn signal_existing(socket_path: &Path) {
    for attempt in 0..SIGNAL_ATTEMPTS {
        if let Ok(client) = UnixDatagram::unbound()
            && client.connect(socket_path).is_ok()
            && client.send(UI_SHOW_SIGNAL).is_ok()
        {
            return;
        }
        if attempt + 1 < SIGNAL_ATTEMPTS {
            thread::sleep(SIGNAL_RETRY_DELAY);
        }
    }
}

fn install(socket: UnixDatagram) -> bool {
    let _ = socket.set_nonblocking(true);
    UI_INSTANCE.set(socket).is_ok()
}

pub(crate) fn drain_show_requests() -> bool {
    let Some(socket) = UI_INSTANCE.get() else {
        return false;
    };
    let mut show_requested = false;
    let mut buffer = [0_u8; 16];
    loop {
        match socket.recv(&mut buffer) {
            Ok(length) if is_show_signal(&buffer[..length]) => show_requested = true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    show_requested
}

fn is_show_signal(message: &[u8]) -> bool {
    message == UI_SHOW_SIGNAL
}

pub(crate) fn release() {
    if let Ok(paths) = AppPaths::discover() {
        let _ = std::fs::remove_file(paths.support_dir.join("ui.sock"));
    }
}

#[cfg(test)]
#[path = "../tests/unit/instance.rs"]
mod tests;
