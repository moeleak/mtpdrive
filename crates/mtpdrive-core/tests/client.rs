use mtpdrive_core::{
    ControlRequest, ControlResponse, DaemonClient, DaemonRequestError, DeviceSummary, Error,
    LogLevel, LogRecord,
};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

fn mock_client(
    expected: ControlRequest,
    response: ControlResponse,
) -> (TempDir, DaemonClient, JoinHandle<()>) {
    let directory = tempfile::tempdir().expect("temporary socket directory");
    let socket_path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind mock daemon");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept request");
        let (read_half, mut write_half) = stream.into_split();
        let mut line = String::new();
        BufReader::new(read_half)
            .read_line(&mut line)
            .await
            .expect("read request");
        assert_eq!(
            serde_json::from_str::<ControlRequest>(&line).expect("decode request"),
            expected
        );
        let mut encoded = serde_json::to_vec(&response).expect("encode response");
        encoded.push(b'\n');
        write_half
            .write_all(&encoded)
            .await
            .expect("write response");
    });
    (directory, DaemonClient::new(socket_path), server)
}

#[tokio::test]
async fn typed_device_request_returns_devices() {
    let devices = vec![DeviceSummary {
        key: "device".to_owned(),
        manufacturer: "Google".to_owned(),
        model: "Pixel".to_owned(),
        serial: "serial".to_owned(),
        device_version: "1".to_owned(),
        usb_speed: Some("USB High".to_owned()),
        generation: 1,
        writable: true,
        storages: Vec::new(),
    }];
    let (_directory, client, server) = mock_client(
        ControlRequest::Devices,
        ControlResponse::Devices(devices.clone()),
    );

    assert_eq!(client.devices().await.expect("typed response"), devices);
    server.await.expect("mock daemon task");
}

#[tokio::test]
async fn typed_log_request_preserves_request_parameters() {
    let records = vec![LogRecord {
        id: 7,
        unix_millis: 123,
        level: LogLevel::Info,
        target: "test".to_owned(),
        message: "ready".to_owned(),
    }];
    let (_directory, client, server) = mock_client(
        ControlRequest::Logs {
            after: 5,
            limit: 20,
        },
        ControlResponse::Logs(records.clone()),
    );

    assert_eq!(client.logs(5, 20).await.expect("typed response"), records);
    server.await.expect("mock daemon task");
}

#[tokio::test]
async fn typed_requests_preserve_daemon_and_unexpected_errors() {
    let (_directory, client, server) = mock_client(
        ControlRequest::Mount,
        ControlResponse::Error {
            message: "mount failed".to_owned(),
        },
    );
    assert!(matches!(
        client.mount().await,
        Err(DaemonRequestError::Daemon(message)) if message == "mount failed"
    ));
    server.await.expect("mock daemon task");

    let (_directory, client, server) = mock_client(ControlRequest::Devices, ControlResponse::Ok);
    assert!(matches!(
        client.devices().await,
        Err(DaemonRequestError::Unexpected(ControlResponse::Ok))
    ));
    server.await.expect("mock daemon task");
}

#[tokio::test]
async fn typed_requests_preserve_transport_errors() {
    let directory = tempfile::tempdir().expect("temporary socket directory");
    let client = DaemonClient::new(directory.path().join("missing.sock"));

    assert!(matches!(
        client.devices().await,
        Err(DaemonRequestError::Client(Error::DaemonUnavailable))
    ));
}
