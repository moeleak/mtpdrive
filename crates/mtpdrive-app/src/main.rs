#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod tray_template;

use directories::UserDirs;
use iced::futures::SinkExt;
use iced::time::Instant;
use iced::widget::{Column, Container, Row, Space};
use iced::{Length, Size, Subscription, Task};
use material::widget::{
    button, container, log_viewer, navigation, page, progress_bar, select, toggler,
};
use material_ui_rs as material;
use mtpdrive_core::{
    AppPaths, AppSettings, ControlRequest, ControlResponse, DaemonClient, DeviceSummary, Language,
    LanguagePreference, LogRecord, MountState, ServiceSnapshot, current_language,
    set_current_language,
};
use semver::Version;
use std::fmt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const WINDOW_SIZE: Size = Size::new(920.0, 720.0);
const MIN_WINDOW_SIZE: Size = Size::new(620.0, 520.0);
const MENU_SHOW: &str = "mtpdrive.show";
const MENU_OPEN: &str = "mtpdrive.open";
const MENU_REFRESH: &str = "mtpdrive.refresh";
const MENU_QUIT: &str = "mtpdrive.quit";
static UI_INSTANCE: OnceLock<UnixDatagram> = OnceLock::new();

/// Runs the `MTPDrive` menu bar application.
///
/// # Errors
///
/// Returns an error when the graphical runtime cannot be initialized.
pub fn main() -> iced::Result {
    if !acquire_ui_instance() {
        return Ok(());
    }
    let result = material::application(boot, update, view)
        .title("MTPDrive")
        .subscription(subscription)
        .window(window_settings())
        .exit_on_close_request(false)
        .run();
    release_ui_instance();
    result
}

fn window_settings() -> iced::window::Settings {
    let mut settings = material::window_with_min_size(WINDOW_SIZE, MIN_WINDOW_SIZE);
    settings.visible = false;
    settings
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    MenuPressed,
    WindowResized(Size),
    CloseRequested(iced::window::Id),
    Frame(Instant),
    Tick,
    DaemonReady(Result<(), String>),
    ServiceUpdated {
        snapshot: Result<ServiceSnapshot, String>,
        logs: Result<Vec<LogRecord>, String>,
    },
    Mount,
    OpenFinder,
    RefreshDevices,
    AlwaysOpenChanged(bool),
    LanguageChanged(LanguagePreference),
    CheckForUpdates,
    UpdateChecked(Result<ReleaseCheck, String>),
    DownloadUpdate {
        version: String,
        asset: ReleaseAsset,
    },
    DownloadEvent(DownloadEvent),
    ControlFinished(Result<ControlResponse, String>),
    LogViewer(log_viewer::Action<u64>),
    ClearLogView,
    ShowWindow,
    WindowLocated(Option<iced::window::Id>),
    Quit,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Devices,
    Logs,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LanguageOption {
    preference: LanguagePreference,
    label: &'static str,
}

impl fmt::Display for LanguageOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label)
    }
}

#[derive(Debug, Clone)]
struct ReleaseCheck {
    latest_version: String,
    update_available: bool,
    asset: Option<ReleaseAsset>,
}

#[derive(Debug, Clone)]
struct ReleaseAsset {
    name: String,
    url: String,
    size: u64,
}

#[derive(Debug, Clone)]
enum DownloadEvent {
    Progress(u64),
    Verifying,
    Finished(Result<PathBuf, String>),
}

#[derive(Debug, Clone, Default)]
enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    CheckFailed(String),
    Downloading {
        version: String,
        asset: ReleaseAsset,
        downloaded: u64,
    },
    Verifying {
        version: String,
        asset: ReleaseAsset,
    },
    Downloaded(PathBuf),
    DownloadFailed {
        version: String,
        asset: ReleaseAsset,
        error: String,
    },
}

struct App {
    language: Language,
    settings: AppSettings,
    language_options: [LanguageOption; 3],
    update_state: UpdateState,
    destinations: [navigation::Destination<Page>; 3],
    navigation: navigation::NavigationState<Page>,
    window_size: Size,
    snapshot: ServiceSnapshot,
    logs: Vec<LogRecord>,
    log_entries: Vec<log_viewer::LogEntry<u64>>,
    log_viewer: log_viewer::State<u64>,
    progress_animation: progress_bar::IndeterminateState,
    last_log_id: u64,
    error: Option<String>,
    service_ready: bool,
    tray: Option<TrayIcon>,
}

fn boot() -> (App, Task<Message>) {
    let (settings, settings_error) =
        match AppPaths::discover().and_then(|paths| AppSettings::load(&paths)) {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(error)),
        };
    let language = settings.language.resolve();
    set_current_language(language);
    let (tray, tray_error) = match create_tray(language) {
        Ok(tray) => (Some(tray), None),
        Err(error) => (None, Some(language.tray_creation_failed(error))),
    };
    let app = App {
        language,
        settings,
        language_options: language_options(language),
        update_state: UpdateState::default(),
        destinations: destinations(language),
        navigation: navigation::NavigationState::new(Page::Devices),
        window_size: WINDOW_SIZE,
        snapshot: ServiceSnapshot::default(),
        logs: Vec::new(),
        log_entries: Vec::new(),
        log_viewer: log_viewer::State::new(),
        progress_animation: progress_bar::IndeterminateState::new(Instant::now()),
        last_log_id: 0,
        error: settings_error.map(|error| error.to_string()).or(tray_error),
        service_ready: false,
        tray,
    };
    (
        app,
        Task::perform(ensure_daemon(language), Message::DaemonReady),
    )
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(destination) => {
            app.navigation.select(
                destination,
                Instant::now(),
                navigation::adaptive_layout(app.window_size.width, app.window_size.height),
            );
            Task::none()
        }
        Message::MenuPressed => {
            app.navigation.toggle_menu_now();
            Task::none()
        }
        Message::WindowResized(size) => {
            app.window_size = size;
            Task::none()
        }
        Message::CloseRequested(id) => iced::window::set_mode(id, iced::window::Mode::Hidden),
        Message::Frame(now) => {
            let _ = app.navigation.advance(now);
            let _ = app.log_viewer.advance(now);
            app.progress_animation.advance(now);
            Task::none()
        }
        Message::DaemonReady(result) => {
            match result {
                Ok(()) => {
                    app.service_ready = true;
                    app.error = None;
                }
                Err(error) => app.error = Some(error),
            }
            refresh_task(app.last_log_id)
        }
        Message::Tick => {
            let mut tasks = tray_tasks();
            if drain_ui_instance_signals() {
                tasks.push(Task::done(Message::ShowWindow));
            }
            if app.service_ready {
                tasks.push(refresh_task(app.last_log_id));
            }
            Task::batch(tasks)
        }
        Message::ServiceUpdated { snapshot, logs } => {
            match snapshot {
                Ok(snapshot) => {
                    app.snapshot = snapshot;
                    app.service_ready = true;
                    if app.error.as_deref().is_some_and(is_service_error) {
                        app.error = None;
                    }
                }
                Err(error) => {
                    app.service_ready = false;
                    app.error = Some(error);
                }
            }
            if let Ok(records) = logs {
                for record in records {
                    if record.id > app.last_log_id {
                        app.last_log_id = record.id;
                        app.logs.push(record);
                    }
                }
                if app.logs.len() > 10_000 {
                    let remove = app.logs.len() - 10_000;
                    app.logs.drain(..remove);
                }
                rebuild_log_entries(app);
            }
            Task::none()
        }
        Message::Mount => control_task(ControlRequest::Mount),
        Message::OpenFinder => control_task(ControlRequest::Open),
        Message::RefreshDevices => control_task(ControlRequest::Refresh),
        Message::AlwaysOpenChanged(always_open_in_finder) => {
            app.settings.always_open_in_finder = always_open_in_finder;
            control_task(ControlRequest::SetSettings {
                settings: app.settings,
            })
        }
        Message::LanguageChanged(language_preference) => {
            app.settings.language = language_preference;
            let language = language_preference.resolve();
            set_current_language(language);
            app.language = language;
            app.destinations = destinations(language);
            app.language_options = language_options(language);
            match create_tray(language) {
                Ok(tray) => app.tray = Some(tray),
                Err(error) => app.error = Some(language.tray_creation_failed(error)),
            }
            control_task(ControlRequest::SetSettings {
                settings: app.settings,
            })
        }
        Message::CheckForUpdates => {
            app.update_state = UpdateState::Checking;
            Task::perform(check_for_updates(), Message::UpdateChecked)
        }
        Message::UpdateChecked(result) => match result {
            Ok(release) if release.update_available => {
                if let Some(asset) = release.asset {
                    app.update_state = UpdateState::Downloading {
                        version: release.latest_version,
                        asset: asset.clone(),
                        downloaded: 0,
                    };
                    download_task(asset)
                } else {
                    app.update_state = UpdateState::CheckFailed(
                        "the release does not contain a downloadable DMG".to_owned(),
                    );
                    Task::none()
                }
            }
            Ok(_) => {
                app.update_state = UpdateState::UpToDate;
                Task::none()
            }
            Err(error) => {
                app.update_state = UpdateState::CheckFailed(error);
                Task::none()
            }
        },
        Message::DownloadUpdate { version, asset } => {
            app.update_state = UpdateState::Downloading {
                version,
                asset: asset.clone(),
                downloaded: 0,
            };
            download_task(asset)
        }
        Message::DownloadEvent(DownloadEvent::Progress(downloaded)) => {
            if let UpdateState::Downloading {
                downloaded: current,
                ..
            } = &mut app.update_state
            {
                *current = downloaded;
            }
            Task::none()
        }
        Message::DownloadEvent(DownloadEvent::Verifying) => {
            if let UpdateState::Downloading { version, asset, .. } = &app.update_state {
                app.update_state = UpdateState::Verifying {
                    version: version.clone(),
                    asset: asset.clone(),
                };
            }
            Task::none()
        }
        Message::DownloadEvent(DownloadEvent::Finished(result)) => {
            app.update_state = match result {
                Ok(path) => UpdateState::Downloaded(path),
                Err(error) => match &app.update_state {
                    UpdateState::Downloading { version, asset, .. }
                    | UpdateState::Verifying { version, asset } => UpdateState::DownloadFailed {
                        version: version.clone(),
                        asset: asset.clone(),
                        error,
                    },
                    _ => UpdateState::CheckFailed(error),
                },
            };
            Task::none()
        }
        Message::ControlFinished(result) => {
            match result {
                Ok(ControlResponse::Error { message }) | Err(message) => app.error = Some(message),
                Ok(_) => app.error = None,
            }
            refresh_task(app.last_log_id)
        }
        Message::LogViewer(action) => app.log_viewer.update(action, &app.log_entries),
        Message::ClearLogView => {
            app.logs.clear();
            app.log_entries.clear();
            app.log_viewer.clear_selection();
            Task::none()
        }
        Message::ShowWindow => iced::window::latest().map(Message::WindowLocated),
        Message::WindowLocated(Some(id)) => Task::batch([
            iced::window::set_mode(id, iced::window::Mode::Windowed),
            iced::window::gain_focus(id),
        ]),
        Message::WindowLocated(None) => Task::none(),
        Message::Quit => Task::perform(shutdown_daemon(), |()| Message::Exit),
        Message::Exit => iced::exit(),
    }
}

fn subscription(app: &App) -> Subscription<Message> {
    let mut subscriptions = vec![
        iced::time::every(Duration::from_millis(750)).map(|_| Message::Tick),
        iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
        iced::window::close_requests().map(Message::CloseRequested),
    ];
    let progress_is_visible = (app.navigation.selected() == Page::Devices
        && app
            .snapshot
            .devices
            .iter()
            .any(|device| !device.storages.is_empty()))
        || (app.navigation.selected() == Page::Settings
            && matches!(
                &app.update_state,
                UpdateState::Checking
                    | UpdateState::Downloading { .. }
                    | UpdateState::Verifying { .. }
            ));
    if app.navigation.is_animating()
        || app.log_viewer.is_animating()
        || (progress_is_visible && app.progress_animation.is_animating())
    {
        subscriptions.push(iced::window::frames().map(Message::Frame));
    }
    Subscription::batch(subscriptions)
}

fn view(app: &App) -> material::Element<'_, Message> {
    let page_content = match app.navigation.selected() {
        Page::Devices => devices_page(app),
        Page::Logs => logs_page(app),
        Page::Settings => settings_page(app),
    };
    navigation::suite(&app.destinations, &app.navigation)
        .layout(navigation::adaptive_layout(
            app.window_size.width,
            app.window_size.height,
        ))
        .with_menu("MTPDrive", Message::MenuPressed)
        .view(Message::Navigate, page_content)
}

fn devices_page(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let mount_description = match &app.snapshot.mount {
        MountState::Unmounted => strings.unmounted.to_owned(),
        MountState::Mounting => strings.mounting.to_owned(),
        MountState::Mounted { path, .. } => app.language.mounted_at(path.display()),
        MountState::Error { message } => app.language.mount_failed(message),
    };

    let mut action_items: Vec<material::Element<'_, Message>> = Vec::with_capacity(3);
    if !matches!(app.snapshot.mount, MountState::Mounted { .. }) {
        action_items.push(
            button::button(strings.mount, button::ButtonVariant::Filled)
                .on_press(Message::Mount)
                .into(),
        );
    }
    action_items.extend([
        button::button(strings.open_in_finder, button::ButtonVariant::FilledTonal)
            .on_press(Message::OpenFinder)
            .into(),
        button::button(strings.rescan, button::ButtonVariant::Text)
            .on_press(Message::RefreshDevices)
            .into(),
    ]);
    let actions = page::row(action_items);

    let status = container::filled_card(
        Column::new()
            .push(material::text::headline_medium(strings.network_volume))
            .push(material::text::body_large(mount_description))
            .push(actions)
            .spacing(12),
    )
    .padding(18)
    .width(Length::Fill);

    let mut body = Column::new().push(status).spacing(16).width(Length::Fill);
    if let Some(error) = &app.error {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::title_medium(strings.action_required))
                    .push(material::text::body_medium(error)),
            )
            .padding(16)
            .width(Length::Fill),
        );
    }
    if let Some(error) = &app.snapshot.last_error {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::title_medium(strings.device_action_required))
                    .push(material::text::body_medium(error)),
            )
            .padding(16)
            .width(Length::Fill),
        );
    }
    if app.snapshot.devices.is_empty() {
        body = body.push(
            container::outlined_card(
                Column::new()
                    .push(material::text::headline_medium(strings.no_devices))
                    .push(material::text::body_large(strings.connect_android))
                    .spacing(6),
            )
            .padding(22)
            .width(Length::Fill),
        );
    } else {
        let progress_phase = app.progress_animation.linear_phase();
        for device in &app.snapshot.devices {
            body = body.push(device_card(app.language, device, progress_phase));
        }
    }

    page::surface(
        page::header(
            strings.devices,
            app.language.device_count(app.snapshot.devices.len()),
        ),
        body,
    )
    .into()
}

fn device_card(
    language: Language,
    device: &DeviceSummary,
    progress_phase: f32,
) -> material::Element<'_, Message> {
    let strings = language.strings();
    let mut content = Column::new()
        .push(material::text::headline_medium(format!(
            "{} {}",
            device.manufacturer, device.model
        )))
        .push(material::text::body_medium(language.device_details(
            &device.serial,
            device.usb_speed.as_deref().unwrap_or(strings.unknown),
            device.writable,
        )))
        .spacing(8)
        .width(Length::Fill);

    for storage in &device.storages {
        let used = storage.total_bytes.saturating_sub(storage.free_bytes);
        let ratio = if storage.total_bytes == 0 {
            0.0
        } else {
            progress_ratio(used, storage.total_bytes)
        };
        content = content.push(
            Column::new()
                .push(
                    Row::new()
                        .push(material::text::title_medium(&storage.name))
                        .push(Space::new().width(Length::Fill))
                        .push(material::text::body_medium(language.storage_capacity(
                            &format_bytes(storage.free_bytes),
                            &format_bytes(storage.total_bytes),
                        ))),
                )
                .push(progress_bar::linear(
                    progress_bar::LinearProgressMode::determinate(ratio, progress_phase),
                ))
                .spacing(6),
        );
    }
    container::outlined_card(content)
        .padding(20)
        .width(Length::Fill)
        .into()
}

fn logs_page(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let toolbar = Row::new()
        .push(
            Column::new()
                .push(material::text::headline_large(strings.logs))
                .push(material::text::body_large(
                    app.language.recent_log_count(app.log_entries.len()),
                ))
                .spacing(4),
        )
        .push(Space::new().width(Length::Fill))
        .push(
            button::button(strings.clear_view, button::ButtonVariant::Text)
                .on_press(Message::ClearLogView),
        )
        .align_y(iced::Alignment::Center);
    let viewer = log_viewer::view(&app.log_entries, &app.log_viewer, Message::LogViewer)
        .width(Length::Fill)
        .height(Length::Fill);
    Container::new(Column::new().push(toolbar).push(viewer).spacing(18))
        .padding(28)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn settings_page(app: &App) -> material::Element<'_, Message> {
    let strings = app.language.strings();
    let behavior = container::outlined_card(
        Column::new()
            .push(toggler::standard(
                app.settings.always_open_in_finder,
                strings.always_open_in_finder,
                Message::AlwaysOpenChanged,
            ))
            .push(material::text::body_medium(
                strings.always_open_in_finder_description,
            ))
            .spacing(8),
    )
    .padding(20)
    .width(Length::Fill);

    let selected_language = app
        .language_options
        .iter()
        .find(|option| option.preference == app.settings.language);
    let language_picker = select::outlined(
        app.language_options.as_slice(),
        selected_language,
        |option: LanguageOption| Message::LanguageChanged(option.preference),
    )
    .label(strings.language)
    .width(Length::Fixed(320.0));
    let language = container::outlined_card(
        Column::new()
            .push(material::text::title_medium(strings.language))
            .push(material::text::body_medium(strings.language_description))
            .push(language_picker)
            .spacing(10),
    )
    .padding(20)
    .width(Length::Fill);

    let mut update = Column::new()
        .push(material::text::title_medium(strings.about))
        .push(material::text::body_large(
            app.language.current_version(env!("CARGO_PKG_VERSION")),
        ))
        .spacing(10);
    update = match &app.update_state {
        UpdateState::Idle => update.push(
            button::button(
                strings.check_for_updates,
                button::ButtonVariant::FilledTonal,
            )
            .on_press(Message::CheckForUpdates),
        ),
        UpdateState::Checking => update.push(
            Row::new()
                .push(progress_bar::loading(
                    progress_bar::LoadingIndicatorMode::contained_indeterminate(
                        app.progress_animation.loading_phase(),
                    ),
                ))
                .push(material::text::body_medium(strings.checking_for_updates))
                .spacing(12)
                .align_y(iced::Alignment::Center),
        ),
        UpdateState::UpToDate => update
            .push(material::text::body_medium(strings.up_to_date))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::CheckFailed(error) => update
            .push(material::text::body_medium(
                app.language.update_check_failed(error),
            ))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::Downloading {
            asset, downloaded, ..
        } => {
            let progress = if asset.size == 0 {
                0.0
            } else {
                progress_ratio(*downloaded, asset.size)
            };
            update
                .push(material::text::body_medium(
                    app.language
                        .downloading_update(*downloaded, asset.size, progress),
                ))
                .push(progress_bar::linear(
                    progress_bar::LinearProgressMode::determinate(
                        progress,
                        app.progress_animation.linear_phase(),
                    ),
                ))
        }
        UpdateState::Verifying { .. } => update.push(
            Row::new()
                .push(progress_bar::loading(
                    progress_bar::LoadingIndicatorMode::contained_indeterminate(
                        app.progress_animation.loading_phase(),
                    ),
                ))
                .push(material::text::body_medium(strings.verifying_update))
                .spacing(12)
                .align_y(iced::Alignment::Center),
        ),
        UpdateState::Downloaded(path) => update
            .push(material::text::body_medium(
                app.language.update_downloaded(path.display()),
            ))
            .push(
                button::button(strings.check_for_updates, button::ButtonVariant::Text)
                    .on_press(Message::CheckForUpdates),
            ),
        UpdateState::DownloadFailed {
            version,
            asset,
            error,
        } => update
            .push(material::text::body_medium(
                app.language.update_download_failed(error),
            ))
            .push(
                button::button(strings.retry_download, button::ButtonVariant::FilledTonal)
                    .on_press(Message::DownloadUpdate {
                        version: version.clone(),
                        asset: asset.clone(),
                    }),
            ),
    };
    let about = container::outlined_card(update)
        .padding(20)
        .width(Length::Fill);

    page::surface(
        page::header(strings.settings, strings.settings_description),
        Column::new()
            .push(behavior)
            .push(language)
            .push(about)
            .spacing(16)
            .width(Length::Fill),
    )
    .into()
}

fn destinations(language: Language) -> [navigation::Destination<Page>; 3] {
    let strings = language.strings();
    [
        navigation::Destination::new(Page::Devices, "devices", strings.devices),
        navigation::Destination::new(Page::Logs, "description", strings.logs),
        navigation::Destination::new(Page::Settings, "settings", strings.settings),
    ]
}

fn language_options(language: Language) -> [LanguageOption; 3] {
    [
        LanguageOption {
            preference: LanguagePreference::System,
            label: language.strings().system_default,
        },
        LanguageOption {
            preference: LanguagePreference::English,
            label: "English",
        },
        LanguageOption {
            preference: LanguagePreference::SimplifiedChinese,
            label: "简体中文",
        },
    ]
}

async fn check_for_updates() -> Result<ReleaseCheck, String> {
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

fn download_task(asset: ReleaseAsset) -> Task<Message> {
    Task::run(
        iced::stream::channel(8, move |mut output| async move {
            let result = download_dmg(&asset, &mut output).await;
            let _ = output.send(DownloadEvent::Finished(result)).await;
        }),
        Message::DownloadEvent,
    )
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

fn rebuild_log_entries(app: &mut App) {
    app.log_entries = app
        .logs
        .iter()
        .map(|record| {
            log_viewer::LogEntry::new(
                record.id,
                match record.level {
                    mtpdrive_core::LogLevel::Trace => log_viewer::LogLevel::Trace,
                    mtpdrive_core::LogLevel::Debug => log_viewer::LogLevel::Debug,
                    mtpdrive_core::LogLevel::Info => log_viewer::LogLevel::Info,
                    mtpdrive_core::LogLevel::Warn => log_viewer::LogLevel::Warn,
                    mtpdrive_core::LogLevel::Error => log_viewer::LogLevel::Error,
                },
                format!(
                    " [{}] [{}] {}",
                    record.unix_millis, record.target, record.message
                ),
            )
        })
        .collect();
    app.log_viewer.retain_entries(&app.log_entries);
}

fn refresh_task(after: u64) -> Task<Message> {
    Task::perform(fetch_service(after), |(snapshot, logs)| {
        Message::ServiceUpdated { snapshot, logs }
    })
}

fn control_task(request: ControlRequest) -> Task<Message> {
    Task::perform(
        async move {
            DaemonClient::discover()
                .map_err(|error| error.to_string())?
                .request(request)
                .await
                .map_err(|error| error.to_string())
        },
        Message::ControlFinished,
    )
}

async fn fetch_service(
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
    let logs = match client
        .request(ControlRequest::Logs {
            after,
            limit: 2_000,
        })
        .await
    {
        Ok(ControlResponse::Logs(logs)) => Ok(logs),
        Ok(ControlResponse::Error { message }) => Err(message),
        Ok(other) => Err(current_language().invalid_log_response(other)),
        Err(error) => Err(error.to_string()),
    };
    (snapshot, logs)
}

async fn ensure_daemon(language: Language) -> Result<(), String> {
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

    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if client.is_running().await {
            return Ok(());
        }
    }
    Err(language.strings().daemon_start_timeout.to_owned())
}

async fn shutdown_daemon() {
    if let Ok(client) = DaemonClient::discover() {
        let _ = client.request(ControlRequest::Shutdown).await;
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

fn acquire_ui_instance() -> bool {
    let Ok(paths) = AppPaths::discover() else {
        return true;
    };
    if paths.ensure().is_err() {
        return true;
    }
    let socket_path = paths.support_dir.join("ui.sock");
    match UnixDatagram::bind(&socket_path) {
        Ok(socket) => install_ui_instance(socket),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if let Ok(client) = UnixDatagram::unbound()
                && client.connect(&socket_path).is_ok()
                && client.send(b"show").is_ok()
            {
                return false;
            }
            let _ = std::fs::remove_file(&socket_path);
            UnixDatagram::bind(&socket_path).is_ok_and(install_ui_instance)
        }
        Err(_) => true,
    }
}

fn install_ui_instance(socket: UnixDatagram) -> bool {
    let _ = socket.set_nonblocking(true);
    UI_INSTANCE.set(socket).is_ok()
}

fn drain_ui_instance_signals() -> bool {
    let Some(socket) = UI_INSTANCE.get() else {
        return false;
    };
    let mut received = false;
    let mut buffer = [0_u8; 16];
    loop {
        match socket.recv(&mut buffer) {
            Ok(_) => received = true,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    received
}

fn release_ui_instance() {
    if let Ok(paths) = AppPaths::discover() {
        let _ = std::fs::remove_file(paths.support_dir.join("ui.sock"));
    }
}

fn tray_tasks() -> Vec<Task<Message>> {
    let mut tasks = Vec::new();
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            tasks.push(Task::done(Message::ShowWindow));
        }
    }
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        let message = match event.id.0.as_str() {
            MENU_SHOW => Some(Message::ShowWindow),
            MENU_OPEN => Some(Message::OpenFinder),
            MENU_REFRESH => Some(Message::RefreshDevices),
            MENU_QUIT => Some(Message::Quit),
            _ => None,
        };
        if let Some(message) = message {
            tasks.push(Task::done(message));
        }
    }
    tasks
}

fn create_tray(language: Language) -> Result<TrayIcon, String> {
    let strings = language.strings();
    let show = MenuItem::with_id(MENU_SHOW, strings.show_mtpdrive, true, None);
    let open = MenuItem::with_id(MENU_OPEN, strings.open_in_finder, true, None);
    let refresh = MenuItem::with_id(MENU_REFRESH, strings.rescan_devices, true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(MENU_QUIT, strings.quit_mtpdrive, true, None);
    let menu = Menu::with_items(&[&show, &open, &refresh, &separator, &quit])
        .map_err(|error| error.to_string())?;
    TrayIconBuilder::new()
        .with_tooltip("MTPDrive")
        .with_icon(template_icon()?)
        .with_icon_as_template(true)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(true)
        .build()
        .map_err(|error| error.to_string())
}

fn is_service_error(value: &str) -> bool {
    ["daemon", "service", "后台服务", "MTPDrive 服务"]
        .iter()
        .any(|marker| value.contains(marker))
}

fn template_icon() -> Result<Icon, String> {
    Icon::from_rgba(
        tray_template::rgba(),
        tray_template::SIZE,
        tray_template::SIZE,
    )
    .map_err(|error| error.to_string())
}

#[allow(clippy::cast_precision_loss)]
fn progress_ratio(value: u64, total: u64) -> f32 {
    value as f32 / total as f32
}

#[allow(clippy::cast_precision_loss)]
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::parse_release_response;

    const RELEASE: &[u8] = br#"{
        "tag_name": "v1.4.0",
        "assets": [
            {
                "name": "MTPDrive-1.4.0-universal.dmg",
                "size": 12345,
                "browser_download_url": "https://github.com/moeleak/mtpdrive/releases/download/v1.4.0/MTPDrive-1.4.0-universal.dmg"
            }
        ]
    }"#;

    #[test]
    fn release_check_compares_semantic_versions() {
        let update = parse_release_response(RELEASE, "1.3.9").expect("valid release");
        assert!(update.update_available);
        assert_eq!(update.latest_version, "1.4.0");
        assert_eq!(
            update.asset.expect("DMG asset").name,
            "MTPDrive-1.4.0-universal.dmg"
        );

        let current = parse_release_response(RELEASE, "1.4.1").expect("valid release");
        assert!(!current.update_available);
    }

    #[test]
    fn release_check_rejects_untrusted_urls() {
        let response = br#"{
            "tag_name": "v9.0.0",
            "assets": [{
                "name": "MTPDrive-9.0.0-universal.dmg",
                "size": 12345,
                "browser_download_url": "https://example.com/not-mtpdrive.dmg"
            }]
        }"#;
        assert!(parse_release_response(response, "1.0.0").is_err());
    }
}
