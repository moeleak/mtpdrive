use mtpdrive_core::{
    ControlRequest, ControlResponse, DaemonClient, DaemonRequestError, Language, LogRecord,
    ServiceSnapshot, current_language,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub(crate) async fn control(request: ControlRequest) -> Result<ControlResponse, String> {
    DaemonClient::discover()
        .map_err(|error| error.to_string())?
        .request(request)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn fetch(
    after: u64,
) -> (
    Result<ServiceSnapshot, String>,
    Result<Vec<LogRecord>, String>,
) {
    let client = match DaemonClient::discover() {
        Ok(client) => client,
        Err(error) => return (Err(error.to_string()), Err(error.to_string())),
    };
    let snapshot = client.snapshot().await.map_err(|error| error.to_string());
    let logs = client.logs(after, 2_000).await.map_err(format_log_error);
    (snapshot, logs)
}

fn format_log_error(error: DaemonRequestError) -> String {
    match error {
        DaemonRequestError::Unexpected(response) => {
            current_language().invalid_log_response(response)
        }
        other => other.to_string(),
    }
}

pub(crate) async fn ensure_daemon(language: Language) -> Result<(), String> {
    let client = DaemonClient::discover().map_err(|error| error.to_string())?;
    if client.is_running().await {
        return Ok(());
    }
    let executable =
        daemon_executable().ok_or_else(|| language.strings().daemon_program_missing.to_owned())?;
    Command::new(&executable)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| language.daemon_start_failed(executable.display(), error))?;

    if client
        .wait_until_running(Duration::from_secs(5), Duration::from_millis(100))
        .await
    {
        Ok(())
    } else {
        Err(language.strings().daemon_start_timeout.to_owned())
    }
}

pub(crate) async fn shutdown_daemon() {
    if let Ok(client) = DaemonClient::discover() {
        let _ = client.shutdown().await;
    }
}

fn daemon_executable() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let parent = current.parent()?;
    let bundled = parent.join("../Helpers/mtpdrive");
    if bundled.is_file() {
        return Some(bundled);
    }
    let adjacent = parent.join("mtpdrive");
    if adjacent.is_file() {
        return Some(adjacent);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join("mtpdrive"))
            .find(|path| path.is_file())
    })
}
