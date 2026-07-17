use super::{devices, log, status};
use mtpdrive_core::{
    DeviceSummary, Language, LogLevel, LogRecord, MountState, ServiceSnapshot, StorageSummary,
};
use std::path::PathBuf;

#[test]
fn status_output_preserves_the_existing_text_format() {
    let snapshot = ServiceSnapshot {
        version: "0.1.2".to_owned(),
        mount: MountState::Mounted {
            path: PathBuf::from("/Users/example/MTPDrive"),
            port: 51_896,
        },
        devices: Vec::new(),
        transfer_count: 0,
        last_error: Some("USB unavailable".to_owned()),
    };
    let mut output = Vec::new();

    status(&mut output, Language::English, &snapshot).expect("render status");

    assert_eq!(
        String::from_utf8(output).expect("UTF-8 output"),
        "MTPDrive 0.1.2\nMount: Mounted at /Users/example/MTPDrive\nDevices: 0\nLast error: USB unavailable\n"
    );
}

#[test]
fn device_output_preserves_storage_indentation_and_units() {
    let device = DeviceSummary {
        key: "device".to_owned(),
        manufacturer: "Google".to_owned(),
        model: "Pixel".to_owned(),
        serial: "ABC".to_owned(),
        device_version: "1".to_owned(),
        usb_speed: None,
        generation: 1,
        writable: true,
        storages: vec![StorageSummary {
            id: 1,
            name: "Internal storage".to_owned(),
            total_bytes: 2 * 1024 * 1024,
            free_bytes: 1024,
            writable: false,
        }],
    };
    let mut output = Vec::new();

    devices(&mut output, Language::English, &[device]).expect("render devices");

    assert_eq!(
        String::from_utf8(output).expect("UTF-8 output"),
        "Google Pixel  serial=ABC  writable=yes\n  Internal storage  free=1.0 KiB  total=2.0 MiB  writable=no\n"
    );
}

#[test]
fn log_output_supports_text_and_json_lines() {
    let record = LogRecord {
        id: 7,
        unix_millis: 123,
        level: LogLevel::Info,
        target: "mtp".to_owned(),
        message: "connected".to_owned(),
    };
    let mut text = Vec::new();
    let mut json = Vec::new();

    log(&mut text, &record, false).expect("render text log");
    log(&mut json, &record, true).expect("render JSON log");

    assert_eq!(
        String::from_utf8(text).expect("UTF-8 output"),
        "123 Info mtp        connected\n"
    );
    assert_eq!(
        serde_json::from_slice::<LogRecord>(&json).expect("JSON log"),
        record
    );
}
