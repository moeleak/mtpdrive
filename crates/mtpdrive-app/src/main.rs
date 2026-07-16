#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use iced::time::Instant;
use iced::widget::{Column, Container, Row, Space};
use iced::{Length, Size, Subscription, Task};
use material::widget::{button, container, log_viewer, navigation, page, progress_bar};
use material_ui_rs as material;
use mtpdrive_core::{
    AppPaths, ControlRequest, ControlResponse, DaemonClient, DeviceSummary, LogRecord, MountState,
    ServiceSnapshot,
};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const WINDOW_SIZE: Size = Size::new(920.0, 720.0);
const MIN_WINDOW_SIZE: Size = Size::new(620.0, 520.0);
const MENU_SHOW: &str = "mtpdrive.show";
const MENU_OPEN: &str = "mtpdrive.open";
const MENU_REFRESH: &str = "mtpdrive.refresh";
const MENU_QUIT: &str = "mtpdrive.quit";
static UI_INSTANCE: OnceLock<UnixDatagram> = OnceLock::new();

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
    Unmount,
    OpenFinder,
    RefreshDevices,
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
}

const DESTINATIONS: [navigation::Destination<Page>; 2] = [
    navigation::Destination::new(Page::Devices, "devices", "设备"),
    navigation::Destination::new(Page::Logs, "description", "日志"),
];

struct App {
    navigation: navigation::NavigationState<Page>,
    window_size: Size,
    snapshot: ServiceSnapshot,
    logs: Vec<LogRecord>,
    log_entries: Vec<log_viewer::LogEntry<u64>>,
    log_viewer: log_viewer::State<u64>,
    last_log_id: u64,
    error: Option<String>,
    service_ready: bool,
    _tray: Option<TrayIcon>,
}

fn boot() -> (App, Task<Message>) {
    let (tray, tray_error) = match create_tray() {
        Ok(tray) => (Some(tray), None),
        Err(error) => (None, Some(format!("无法创建菜单栏图标：{error}"))),
    };
    let app = App {
        navigation: navigation::NavigationState::new(Page::Devices),
        window_size: WINDOW_SIZE,
        snapshot: ServiceSnapshot::default(),
        logs: Vec::new(),
        log_entries: Vec::new(),
        log_viewer: log_viewer::State::new(),
        last_log_id: 0,
        error: tray_error,
        service_ready: false,
        _tray: tray,
    };
    (app, Task::perform(ensure_daemon(), Message::DaemonReady))
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
                    if app
                        .error
                        .as_deref()
                        .is_some_and(|value| value.contains("后台服务") || value.contains("daemon"))
                    {
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
        Message::Unmount => control_task(ControlRequest::Unmount),
        Message::OpenFinder => control_task(ControlRequest::Open),
        Message::RefreshDevices => control_task(ControlRequest::Refresh),
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
        Message::Quit => Task::perform(shutdown_daemon(), |_| Message::Exit),
        Message::Exit => iced::exit(),
    }
}

fn subscription(app: &App) -> Subscription<Message> {
    let mut subscriptions = vec![
        iced::time::every(Duration::from_millis(750)).map(|_| Message::Tick),
        iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
        iced::window::close_requests().map(Message::CloseRequested),
    ];
    if app.navigation.is_animating() || app.log_viewer.is_animating() {
        subscriptions.push(iced::window::frames().map(Message::Frame));
    }
    Subscription::batch(subscriptions)
}

fn view(app: &App) -> material::Element<'_, Message> {
    let page_content = match app.navigation.selected() {
        Page::Devices => devices_page(app),
        Page::Logs => logs_page(app),
    };
    navigation::suite(&DESTINATIONS, &app.navigation)
        .layout(navigation::adaptive_layout(
            app.window_size.width,
            app.window_size.height,
        ))
        .with_menu("MTPDrive", Message::MenuPressed)
        .view(Message::Navigate, page_content)
}

fn devices_page(app: &App) -> material::Element<'_, Message> {
    let mount_description = match &app.snapshot.mount {
        MountState::Unmounted => "未挂载".to_owned(),
        MountState::Mounting => "正在挂载…".to_owned(),
        MountState::Mounted { path, .. } => format!("已挂载到 {}", path.display()),
        MountState::Error { message } => format!("挂载失败：{message}"),
    };

    let mount_button: material::Element<'_, Message> = match app.snapshot.mount {
        MountState::Mounted { .. } => button::button("卸载", button::ButtonVariant::Outlined)
            .on_press(Message::Unmount)
            .into(),
        _ => button::button("挂载", button::ButtonVariant::Filled)
            .on_press(Message::Mount)
            .into(),
    };
    let actions = page::row([
        mount_button,
        button::button("在访达中打开", button::ButtonVariant::FilledTonal)
            .on_press(Message::OpenFinder)
            .into(),
        button::button("重新扫描", button::ButtonVariant::Text)
            .on_press(Message::RefreshDevices)
            .into(),
    ]);

    let status = container::filled_card(
        Column::new()
            .push(material::text::headline_medium("MTPDrive 网络卷"))
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
                    .push(material::text::title_medium("需要处理"))
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
                    .push(material::text::title_medium("设备需要处理"))
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
                    .push(material::text::headline_medium("未发现可用设备"))
                    .push(material::text::body_large(
                        "用 USB 连接 Android，并在手机上选择“文件传输 / Android Auto”。",
                    ))
                    .spacing(6),
            )
            .padding(22)
            .width(Length::Fill),
        );
    } else {
        for device in &app.snapshot.devices {
            body = body.push(device_card(device));
        }
    }

    page::surface(
        page::header(
            "设备",
            format!("{} 台 Android MTP 设备", app.snapshot.devices.len()),
        ),
        body,
    )
    .into()
}

fn device_card(device: &DeviceSummary) -> material::Element<'_, Message> {
    let mut content = Column::new()
        .push(material::text::headline_medium(format!(
            "{} {}",
            device.manufacturer, device.model
        )))
        .push(material::text::body_medium(format!(
            "序列号 {}  ·  USB {}  ·  {}",
            device.serial,
            device.usb_speed.as_deref().unwrap_or("未知"),
            if device.writable {
                "可读写"
            } else {
                "只读"
            }
        )))
        .spacing(8)
        .width(Length::Fill);

    for storage in &device.storages {
        let used = storage.total_bytes.saturating_sub(storage.free_bytes);
        let ratio = if storage.total_bytes == 0 {
            0.0
        } else {
            used as f32 / storage.total_bytes as f32
        };
        content = content.push(
            Column::new()
                .push(
                    Row::new()
                        .push(material::text::title_medium(&storage.name))
                        .push(Space::new().width(Length::Fill))
                        .push(material::text::body_medium(format!(
                            "{} 可用，共 {}",
                            format_bytes(storage.free_bytes),
                            format_bytes(storage.total_bytes)
                        ))),
                )
                .push(progress_bar::linear(
                    progress_bar::LinearProgressMode::determinate(ratio, 0.0),
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
    let toolbar = Row::new()
        .push(
            Column::new()
                .push(material::text::headline_large("日志"))
                .push(material::text::body_large(format!(
                    "后台服务的最近 {} 条结构化记录",
                    app.log_entries.len()
                )))
                .spacing(4),
        )
        .push(Space::new().width(Length::Fill))
        .push(
            button::button("清空视图", button::ButtonVariant::Text).on_press(Message::ClearLogView),
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
        Ok(other) => Err(format!("后台日志响应无效：{other:?}")),
        Err(error) => Err(error.to_string()),
    };
    (snapshot, logs)
}

async fn ensure_daemon() -> Result<(), String> {
    let client = DaemonClient::discover().map_err(|error| error.to_string())?;
    if client.is_running().await {
        return Ok(());
    }
    let executable = daemon_executable().ok_or_else(|| {
        "找不到后台服务程序；请重新安装 MTPDrive，或在开发目录先构建 mtpdrive-cli".to_owned()
    })?;
    Command::new(&executable)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法启动后台服务 {}：{error}", executable.display()))?;

    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if client.is_running().await {
            return Ok(());
        }
    }
    Err("后台服务启动超时".to_owned())
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

fn create_tray() -> Result<TrayIcon, String> {
    let show = MenuItem::with_id(MENU_SHOW, "显示 MTPDrive", true, None);
    let open = MenuItem::with_id(MENU_OPEN, "在访达中打开", true, None);
    let refresh = MenuItem::with_id(MENU_REFRESH, "重新扫描设备", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(MENU_QUIT, "退出 MTPDrive", true, None);
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

fn template_icon() -> Result<Icon, String> {
    let width = 32_u32;
    let height = 32_u32;
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    let mut pixel = |x: u32, y: u32, alpha: u8| {
        let index = ((y * width + x) * 4) as usize;
        rgba[index..index + 4].copy_from_slice(&[0, 0, 0, alpha]);
    };
    for y in 9..24 {
        for x in 7..25 {
            let head = y <= 14 && (x as i32 - 16).pow(2) + (y as i32 - 15).pow(2) <= 92;
            let body = (13..=23).contains(&y) && (8..=23).contains(&x);
            if head || body {
                pixel(x, y, 255);
            }
        }
    }
    for step in 0..6 {
        pixel(10 - step / 2, 8 - step, 255);
        pixel(21 + step / 2, 8 - step, 255);
    }
    for y in 20..29 {
        for x in 10..14 {
            pixel(x, y, 255);
        }
        for x in 18..22 {
            pixel(x, y, 255);
        }
    }
    for y in 17..27 {
        for x in 21..31 {
            let distance = (x as i32 - 26).pow(2) + (y as i32 - 22).pow(2);
            if (13..=27).contains(&distance) {
                pixel(x, y, 255);
            }
        }
    }
    Icon::from_rgba(rgba, width, height).map_err(|error| error.to_string())
}

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
