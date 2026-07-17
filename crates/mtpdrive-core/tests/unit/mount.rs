use super::can_reuse_mount;
use crate::MountState;
use std::path::PathBuf;

#[test]
fn daemon_does_not_reuse_a_mount_it_did_not_create() {
    assert!(!can_reuse_mount(&MountState::Unmounted, 51_896, true));
}

#[test]
fn daemon_reuses_its_live_mount_on_the_same_port() {
    let state = MountState::Mounted {
        path: PathBuf::from("/Users/example/MTPDrive"),
        port: 51_896,
    };
    assert!(can_reuse_mount(&state, 51_896, true));
}

#[test]
fn daemon_replaces_a_mount_on_a_different_or_missing_port() {
    let state = MountState::Mounted {
        path: PathBuf::from("/Users/example/MTPDrive"),
        port: 63_250,
    };
    assert!(!can_reuse_mount(&state, 51_896, true));
    assert!(!can_reuse_mount(&state, 63_250, false));
}
