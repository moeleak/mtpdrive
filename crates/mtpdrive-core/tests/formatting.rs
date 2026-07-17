use mtpdrive_core::{Language, MountState, format_bytes, format_mount_state};
use std::path::PathBuf;

#[test]
fn byte_counts_use_existing_binary_unit_format() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(1024), "1.0 KiB");
    assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
}

#[test]
fn mount_states_use_existing_localized_text() {
    let language = Language::English;
    assert_eq!(
        format_mount_state(language, &MountState::Unmounted),
        language.strings().unmounted
    );
    assert_eq!(
        format_mount_state(
            language,
            &MountState::Mounted {
                path: PathBuf::from("/Users/example/MTPDrive"),
                port: 51_896,
            },
        ),
        "Mounted at /Users/example/MTPDrive"
    );
}
