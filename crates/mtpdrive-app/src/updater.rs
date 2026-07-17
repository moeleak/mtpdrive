use directories::UserDirs;
use iced::futures::{SinkExt, Stream};
use semver::Version;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::process::Command as TokioCommand;

#[derive(Debug, Clone)]
pub(crate) struct ReleaseCheck {
    pub(crate) latest_version: String,
    pub(crate) update_available: bool,
    pub(crate) asset: Option<ReleaseAsset>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReleaseAsset {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) size: u64,
}

#[derive(Debug, Clone)]
pub(crate) enum DownloadEvent {
    Progress(u64),
    Verifying,
    Finished(Result<PathBuf, String>),
}

pub(crate) async fn check_for_updates() -> Result<ReleaseCheck, String> {
    tokio::task::spawn_blocking(|| {
        let output = Command::new("/usr/bin/curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--connect-timeout",
                "5",
                "--max-time",
                "10",
                "--header",
                "Accept: application/vnd.github+json",
                "--header",
                "X-GitHub-Api-Version: 2022-11-28",
                "--user-agent",
                concat!("MTPDrive/", env!("CARGO_PKG_VERSION")),
                "https://api.github.com/repos/moeleak/mtpdrive/releases/latest",
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(if detail.is_empty() {
                format!("curl exited with {}", output.status)
            } else {
                detail
            });
        }

        parse_release_response(&output.stdout, env!("CARGO_PKG_VERSION"))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn parse_release_response(response: &[u8], current_version: &str) -> Result<ReleaseCheck, String> {
    let response: serde_json::Value =
        serde_json::from_slice(response).map_err(|error| error.to_string())?;
    let tag = response
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the release did not include a tag name".to_owned())?;
    let latest =
        Version::parse(tag.trim_start_matches(['v', 'V'])).map_err(|error| error.to_string())?;
    let current = Version::parse(current_version).map_err(|error| error.to_string())?;
    let update_available = latest > current;
    let asset = update_available
        .then(|| release_dmg_asset(&response))
        .transpose()?;
    Ok(ReleaseCheck {
        latest_version: latest.to_string(),
        update_available,
        asset,
    })
}

fn release_dmg_asset(response: &serde_json::Value) -> Result<ReleaseAsset, String> {
    response
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?;
            let url = asset.get("browser_download_url")?.as_str()?;
            let size = asset.get("size")?.as_u64()?;
            let is_plain_filename = Path::new(name)
                .file_name()
                .and_then(|filename| filename.to_str())
                == Some(name);
            (is_plain_filename
                && name.to_ascii_lowercase().ends_with(".dmg")
                && url.starts_with("https://github.com/moeleak/mtpdrive/releases/download/")
                && size > 0)
                .then(|| ReleaseAsset {
                    name: name.to_owned(),
                    url: url.to_owned(),
                    size,
                })
        })
        .max_by_key(|asset| asset.name.to_ascii_lowercase().contains("universal"))
        .ok_or_else(|| "the release does not contain a valid DMG asset".to_owned())
}

pub(crate) fn download(asset: ReleaseAsset) -> impl Stream<Item = DownloadEvent> {
    iced::stream::channel(8, move |mut output| async move {
        let result = download_dmg(&asset, &mut output).await;
        let _ = output.send(DownloadEvent::Finished(result)).await;
    })
}

async fn download_dmg(
    asset: &ReleaseAsset,
    output: &mut iced::futures::channel::mpsc::Sender<DownloadEvent>,
) -> Result<PathBuf, String> {
    let downloads = UserDirs::new()
        .and_then(|directories| directories.download_dir().map(Path::to_path_buf))
        .ok_or_else(|| "the Downloads folder could not be found".to_owned())?;
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|error| error.to_string())?;
    let destination = downloads.join(&asset.name);
    let partial = destination.with_extension("dmg.part");
    let _ = tokio::fs::remove_file(&partial).await;

    let result = download_and_verify(asset, &partial, output).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(error);
    }
    tokio::fs::rename(&partial, &destination)
        .await
        .map_err(|error| error.to_string())?;

    let status = TokioCommand::new("/usr/bin/open")
        .arg(&destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("open exited with {status}"));
    }
    Ok(destination)
}

async fn download_and_verify(
    asset: &ReleaseAsset,
    partial: &Path,
    output: &mut iced::futures::channel::mpsc::Sender<DownloadEvent>,
) -> Result<(), String> {
    let mut command = TokioCommand::new("/usr/bin/curl");
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--connect-timeout",
            "10",
            "--output",
        ])
        .arg(partial)
        .arg(&asset.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut last_downloaded = 0;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if let Ok(metadata) = tokio::fs::metadata(partial).await {
            let downloaded = metadata.len().min(asset.size);
            if downloaded != last_downloaded {
                last_downloaded = downloaded;
                output
                    .send(DownloadEvent::Progress(downloaded))
                    .await
                    .map_err(|_| "the download was cancelled".to_owned())?;
            }
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    };
    if !status.success() {
        return Err(format!("curl exited with {status}"));
    }

    let actual_size = tokio::fs::metadata(partial)
        .await
        .map_err(|error| error.to_string())?
        .len();
    if actual_size != asset.size {
        return Err(format!(
            "the DMG size was {actual_size} bytes; expected {} bytes",
            asset.size
        ));
    }
    output
        .send(DownloadEvent::Progress(actual_size))
        .await
        .map_err(|_| "the download was cancelled".to_owned())?;
    output
        .send(DownloadEvent::Verifying)
        .await
        .map_err(|_| "the download was cancelled".to_owned())?;

    let status = TokioCommand::new("/usr/bin/hdiutil")
        .arg("verify")
        .arg(partial)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("hdiutil verify exited with {status}"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/updater.rs"]
mod tests;
