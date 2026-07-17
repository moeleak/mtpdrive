use super::{acquire_lock, is_show_signal, signal_existing};
use std::os::unix::net::UnixDatagram;

#[test]
fn ui_instance_signal_requires_an_exact_show_message() {
    assert!(is_show_signal(b"show"));
    assert!(!is_show_signal(b""));
    assert!(!is_show_signal(b"show\0"));
    assert!(!is_show_signal(b"show-window"));
    assert!(!is_show_signal(b"noise"));
}

#[test]
fn ui_lock_rejects_a_second_owner() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("ui.lock");
    let first = acquire_lock(&path)
        .expect("first lock")
        .expect("lock owner");

    assert!(acquire_lock(&path).expect("second lock").is_none());

    drop(first);
    assert!(acquire_lock(&path).expect("released lock").is_some());
}

#[test]
fn existing_instance_receives_show_signal() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("ui.sock");
    let server = UnixDatagram::bind(&path).expect("bind UI socket");

    signal_existing(&path);

    let mut message = [0_u8; 16];
    let length = server.recv(&mut message).expect("receive show signal");
    assert_eq!(&message[..length], b"show");
}
