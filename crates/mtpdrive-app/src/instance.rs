use mtpdrive_core::AppPaths;
use std::os::unix::net::UnixDatagram;
use std::sync::OnceLock;

const UI_SHOW_SIGNAL: &[u8] = b"show";
static UI_INSTANCE: OnceLock<UnixDatagram> = OnceLock::new();

pub(crate) fn acquire() -> bool {
    let Ok(paths) = AppPaths::discover() else {
        return true;
    };
    if paths.ensure().is_err() {
        return true;
    }
    let socket_path = paths.support_dir.join("ui.sock");
    match UnixDatagram::bind(&socket_path) {
        Ok(socket) => install(socket),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if let Ok(client) = UnixDatagram::unbound()
                && client.connect(&socket_path).is_ok()
                && client.send(UI_SHOW_SIGNAL).is_ok()
            {
                return false;
            }
            let _ = std::fs::remove_file(&socket_path);
            UnixDatagram::bind(&socket_path).is_ok_and(install)
        }
        Err(_) => true,
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
