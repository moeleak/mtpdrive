use crate::WINDOW_SIZE;
use crate::instance;
use crate::login_item;
use crate::service;
use crate::theme;
use crate::tray::{Tray, TrayAction};
use crate::updater::{self, DownloadEvent, ReleaseAsset, ReleaseCheck};
use crate::views::{appearance_options, destinations, language_options};
use iced::time::Instant;
use iced::{Point, Size, Subscription, Task};
use material::widget::{log_viewer, navigation, progress_bar, theme_picker};
use material_ui_rs as material;
use mtpdrive_core::{
    AppPaths, AppSettings, AppearancePreference, ControlRequest, ControlResponse, Language,
    LanguagePreference, LogRecord, ServiceSnapshot, set_current_language,
};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
    OpenAtLoginChanged(bool),
    OpenLoginItemsSettings,
    LanguageChanged(LanguagePreference),
    AppearanceChanged(AppearancePreference),
    ThemeChanged(theme_picker::ThemeAction),
    SystemThemeChanged(iced::theme::Mode),
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
pub(crate) enum Page {
    Devices,
    Logs,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageOption {
    pub(crate) preference: LanguagePreference,
    pub(crate) label: &'static str,
}

impl fmt::Display for LanguageOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppearanceOption {
    pub(crate) preference: AppearancePreference,
    pub(crate) label: &'static str,
}

impl fmt::Display for AppearanceOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) enum UpdateState {
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

pub(crate) struct App {
    pub(crate) language: Language,
    pub(crate) settings: AppSettings,
    pub(crate) language_options: [LanguageOption; 3],
    pub(crate) appearance_options: [AppearanceOption; 3],
    pub(crate) update_state: UpdateState,
    pub(crate) destinations: [navigation::Destination<Page>; 3],
    pub(crate) navigation: navigation::NavigationState<Page>,
    pub(crate) window_size: Size,
    pub(crate) snapshot: ServiceSnapshot,
    pub(crate) log_entries: Vec<log_viewer::LogEntry<u64>>,
    pub(crate) log_viewer: log_viewer::State<u64>,
    pub(crate) progress_animation: progress_bar::IndeterminateState,
    pub(crate) theme_controller: theme_picker::ThemeController,
    pub(crate) login_item_status: login_item::Status,
    pub(crate) login_item_error: Option<String>,
    system_theme: iced::theme::Mode,
    last_log_id: u64,
    pub(crate) error: Option<String>,
    snapshot_poll: SnapshotPoll,
    logs: Vec<LogRecord>,
    tray: Option<Tray>,
}

#[derive(Debug, Default)]
struct SnapshotPoll {
    in_flight: bool,
}

impl SnapshotPoll {
    fn begin(&mut self) -> bool {
        if self.in_flight {
            false
        } else {
            self.in_flight = true;
            true
        }
    }

    fn finish(&mut self) {
        self.in_flight = false;
    }
}

pub(crate) fn boot() -> (App, Task<Message>) {
    let (settings, settings_error) =
        match AppPaths::discover().and_then(|paths| AppSettings::load(&paths)) {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(error)),
        };
    let language = settings.language.resolve();
    let system_theme = iced::theme::Mode::None;
    set_current_language(language);
    let (tray, tray_error) = match Tray::new(language) {
        Ok(tray) => (Some(tray), None),
        Err(error) => (None, Some(language.tray_creation_failed(error))),
    };
    let app = App {
        language,
        settings,
        language_options: language_options(language),
        appearance_options: appearance_options(language),
        update_state: UpdateState::default(),
        destinations: destinations(language),
        navigation: navigation::NavigationState::new(Page::Devices),
        window_size: WINDOW_SIZE,
        snapshot: ServiceSnapshot::default(),
        logs: Vec::new(),
        log_entries: Vec::new(),
        log_viewer: log_viewer::State::new(),
        progress_animation: progress_bar::IndeterminateState::new(Instant::now()),
        theme_controller: theme_picker::ThemeController::new(
            theme::material_color(settings.theme_color),
            theme::effective_dark(settings.appearance, system_theme),
        ),
        login_item_status: login_item::status(),
        login_item_error: None,
        system_theme,
        last_log_id: 0,
        error: settings_error.map(|error| error.to_string()).or(tray_error),
        snapshot_poll: SnapshotPoll::default(),
        tray,
    };
    (
        app,
        Task::batch([
            Task::perform(service::ensure_daemon(language), Message::DaemonReady),
            iced::system::theme().map(Message::SystemThemeChanged),
        ]),
    )
}

pub(crate) fn update(app: &mut App, message: Message) -> Task<Message> {
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
            let _ = app.theme_controller.advance(now);
            let _ = app.navigation.advance(now);
            let _ = app.log_viewer.advance(now);
            app.progress_animation.advance(now);
            Task::none()
        }
        Message::DaemonReady(result) => {
            match result {
                Ok(()) => {
                    app.error = None;
                }
                Err(error) => app.error = Some(error),
            }
            poll_snapshot(app)
        }
        Message::Tick => {
            if app.navigation.selected() == Page::Settings {
                app.login_item_status = login_item::status();
            }
            let mut tasks = Tray::drain_actions(app.tray.as_ref())
                .into_iter()
                .map(|action| Task::done(tray_message(action)))
                .collect::<Vec<_>>();
            if instance::drain_show_requests() {
                tasks.push(Task::done(Message::ShowWindow));
            }
            tasks.push(poll_snapshot(app));
            Task::batch(tasks)
        }
        Message::ServiceUpdated { snapshot, logs } => {
            app.snapshot_poll.finish();
            match snapshot {
                Ok(snapshot) => {
                    app.snapshot = snapshot;
                    if app.error.as_deref().is_some_and(is_service_error) {
                        app.error = None;
                    }
                }
                Err(error) => {
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
        Message::OpenAtLoginChanged(enabled) => {
            match login_item::set_enabled(enabled) {
                Ok(status) => {
                    app.login_item_status = status;
                    app.login_item_error = None;
                }
                Err(error) => {
                    app.login_item_status = login_item::status();
                    app.login_item_error = Some(error);
                }
            }
            Task::none()
        }
        Message::OpenLoginItemsSettings => {
            login_item::open_system_settings();
            Task::none()
        }
        Message::LanguageChanged(language_preference) => {
            app.settings.language = language_preference;
            let language = language_preference.resolve();
            set_current_language(language);
            app.language = language;
            app.destinations = destinations(language);
            app.language_options = language_options(language);
            app.appearance_options = appearance_options(language);
            let tray_result = match app.tray.as_ref() {
                Some(tray) => tray.set_language(language),
                None => Tray::new(language).map(|tray| app.tray = Some(tray)),
            };
            if let Err(error) = tray_result {
                app.error = Some(language.tray_creation_failed(error));
            }
            control_task(ControlRequest::SetSettings {
                settings: app.settings,
            })
        }
        Message::AppearanceChanged(preference) => {
            app.settings.appearance = preference;
            apply_dark_mode(
                app,
                theme::effective_dark(preference, app.system_theme),
                Instant::now(),
            );
            save_settings_task(app)
        }
        Message::ThemeChanged(action) => {
            let should_save = match action {
                theme_picker::ThemeAction::SelectColor(color) => {
                    app.settings.theme_color = theme::persisted_color(color);
                    true
                }
                theme_picker::ThemeAction::SetDarkMode { dark_mode, .. } => {
                    app.settings.appearance = if dark_mode {
                        AppearancePreference::Dark
                    } else {
                        AppearancePreference::Light
                    };
                    true
                }
                theme_picker::ThemeAction::TogglePicker => false,
            };
            app.theme_controller.update(
                action,
                app.window_size,
                picker_bottom_margin(app),
                Instant::now(),
            );
            if should_save {
                save_settings_task(app)
            } else {
                Task::none()
            }
        }
        Message::SystemThemeChanged(mode) => {
            app.system_theme = mode;
            if app.settings.appearance == AppearancePreference::System {
                apply_dark_mode(
                    app,
                    theme::effective_dark(app.settings.appearance, mode),
                    Instant::now(),
                );
            }
            Task::none()
        }
        Message::CheckForUpdates => {
            app.update_state = UpdateState::Checking;
            Task::perform(updater::check_for_updates(), Message::UpdateChecked)
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
            poll_snapshot(app)
        }
        Message::LogViewer(action) => app.log_viewer.update(action, &app.log_entries),
        Message::ClearLogView => {
            app.logs.clear();
            app.log_entries.clear();
            app.log_viewer.clear_selection();
            Task::none()
        }
        Message::ShowWindow => iced::window::latest().map(Message::WindowLocated),
        Message::WindowLocated(Some(id)) => {
            iced::window::set_mode(id, iced::window::Mode::Windowed)
                .chain(iced::window::gain_focus(id))
        }
        Message::WindowLocated(None) => Task::none(),
        Message::Quit => Task::perform(service::shutdown_daemon(), |()| Message::Exit),
        Message::Exit => iced::exit(),
    }
}

pub(crate) fn subscription(app: &App) -> Subscription<Message> {
    let mut subscriptions = vec![
        iced::time::every(Duration::from_millis(750)).map(|_| Message::Tick),
        iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
        iced::window::close_requests().map(Message::CloseRequested),
        iced::system::theme_changes().map(Message::SystemThemeChanged),
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
        || app.theme_controller.is_animating()
        || app.log_viewer.is_animating()
        || (progress_is_visible && app.progress_animation.is_animating())
    {
        subscriptions.push(iced::window::frames().map(Message::Frame));
    }
    Subscription::batch(subscriptions)
}

pub(crate) fn theme(app: &App) -> material::Theme {
    app.theme_controller.theme("MTPDrive")
}

fn tray_message(action: TrayAction) -> Message {
    match action {
        TrayAction::Show => Message::ShowWindow,
        TrayAction::OpenFinder => Message::OpenFinder,
        TrayAction::RefreshDevices => Message::RefreshDevices,
        TrayAction::Quit => Message::Quit,
    }
}

fn download_task(asset: ReleaseAsset) -> Task<Message> {
    Task::run(updater::download(asset), Message::DownloadEvent)
}

fn poll_snapshot(app: &mut App) -> Task<Message> {
    if !app.snapshot_poll.begin() {
        return Task::none();
    }
    let after = app.last_log_id;
    Task::perform(service::fetch(after), |(snapshot, logs)| {
        Message::ServiceUpdated { snapshot, logs }
    })
}

fn control_task(request: ControlRequest) -> Task<Message> {
    Task::perform(service::control(request), Message::ControlFinished)
}

fn save_settings_task(app: &App) -> Task<Message> {
    control_task(ControlRequest::SetSettings {
        settings: app.settings,
    })
}

fn picker_bottom_margin(app: &App) -> f32 {
    theme_picker::bottom_margin(navigation::adaptive_layout(
        app.window_size.width,
        app.window_size.height,
    ))
}

fn apply_dark_mode(app: &mut App, dark_mode: bool, now: Instant) {
    if app.theme_controller.dark_mode() == dark_mode {
        return;
    }
    app.theme_controller.update(
        theme_picker::ThemeAction::SetDarkMode {
            dark_mode,
            origin: Point::new(app.window_size.width / 2.0, app.window_size.height / 2.0),
        },
        app.window_size,
        picker_bottom_margin(app),
        now,
    );
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

fn is_service_error(value: &str) -> bool {
    ["daemon", "service", "后台服务", "MTPDrive 服务"]
        .iter()
        .any(|marker| value.contains(marker))
}

#[cfg(test)]
#[path = "../tests/unit/application.rs"]
mod tests;
