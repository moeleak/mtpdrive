//! Shared localization for the app, daemon, and command-line client.

use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

const APP_DEFAULTS_DOMAIN: &str = "moe.leak.MTPDrive";

static SYSTEM_LANGUAGE: OnceLock<Language> = OnceLock::new();
static CURRENT_LANGUAGE: AtomicU8 = AtomicU8::new(0);

/// Languages currently shipped by `MTPDrive`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    SimplifiedChinese,
}

/// Fixed text shared by the graphical app and command-line client.
#[derive(Debug)]
pub struct Strings {
    pub devices: &'static str,
    pub logs: &'static str,
    pub settings: &'static str,
    pub settings_description: &'static str,
    pub unmounted: &'static str,
    pub mounting: &'static str,
    pub mount: &'static str,
    pub unmount: &'static str,
    pub open_in_finder: &'static str,
    pub rescan: &'static str,
    pub network_volume: &'static str,
    pub action_required: &'static str,
    pub device_action_required: &'static str,
    pub no_devices: &'static str,
    pub connect_android: &'static str,
    pub unknown: &'static str,
    pub read_write: &'static str,
    pub read_only: &'static str,
    pub clear_view: &'static str,
    pub always_open_in_finder: &'static str,
    pub always_open_in_finder_description: &'static str,
    pub open_at_login: &'static str,
    pub open_at_login_description: &'static str,
    pub open_at_login_requires_approval: &'static str,
    pub open_at_login_unavailable: &'static str,
    pub open_login_items_settings: &'static str,
    pub language: &'static str,
    pub language_description: &'static str,
    pub system_default: &'static str,
    pub theme: &'static str,
    pub theme_description: &'static str,
    pub appearance: &'static str,
    pub light: &'static str,
    pub dark: &'static str,
    pub theme_picker_hint: &'static str,
    pub about: &'static str,
    pub check_for_updates: &'static str,
    pub checking_for_updates: &'static str,
    pub up_to_date: &'static str,
    pub verifying_update: &'static str,
    pub retry_download: &'static str,
    pub show_mtpdrive: &'static str,
    pub rescan_devices: &'static str,
    pub quit_mtpdrive: &'static str,
    pub daemon_program_missing: &'static str,
    pub daemon_start_timeout: &'static str,
    pub service_hint: &'static str,
    pub mount_label: &'static str,
    pub devices_label: &'static str,
    pub last_error_label: &'static str,
    pub no_mtp_devices: &'static str,
    pub serial_label: &'static str,
    pub writable_label: &'static str,
    pub free_label: &'static str,
    pub total_label: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
    pub mtp_device: &'static str,
    pub storage: &'static str,
    pub home_directory_unavailable: &'static str,
    pub rename_unsupported: &'static str,
    pub move_unsupported: &'static str,
    pub upload_unsupported: &'static str,
    pub stage_directory_unsupported: &'static str,
    pub write_too_large: &'static str,
    pub empty_response: &'static str,
    pub io_error: &'static str,
    pub mtp_error: &'static str,
    pub serialization_error: &'static str,
    pub daemon_unavailable: &'static str,
    pub invalid_response: &'static str,
    pub disconnected: &'static str,
    pub not_found: &'static str,
    pub unsupported: &'static str,
    pub operation_failed: &'static str,
}

const ENGLISH: Strings = Strings {
    devices: "Devices",
    logs: "Logs",
    settings: "Settings",
    settings_description: "Choose how MTPDrive behaves and check for new versions.",
    unmounted: "Not mounted",
    mounting: "Mounting…",
    mount: "Mount",
    unmount: "Unmount",
    open_in_finder: "Open in Finder",
    rescan: "Rescan",
    network_volume: "MTPDrive network volume",
    action_required: "Action required",
    device_action_required: "Device needs attention",
    no_devices: "No available devices",
    connect_android: "Connect Android over USB, then select “File transfer / Android Auto” on the phone.",
    unknown: "Unknown",
    read_write: "Read & Write",
    read_only: "Read Only",
    clear_view: "Clear view",
    always_open_in_finder: "Always Open in Finder",
    always_open_in_finder_description: "Open the MTPDrive volume when the first Android device connects.",
    open_at_login: "Open at Login",
    open_at_login_description: "Start MTPDrive in the menu bar when you sign in to this Mac.",
    open_at_login_requires_approval: "Allow MTPDrive under Login Items in System Settings to finish enabling this option.",
    open_at_login_unavailable: "Open at Login is only available from an installed, signed MTPDrive app.",
    open_login_items_settings: "Open Login Items Settings",
    language: "Language",
    language_description: "Choose the language used by the app and background service.",
    system_default: "System Default",
    theme: "Theme",
    theme_description: "Choose a Material color theme and how MTPDrive follows the system appearance.",
    appearance: "Appearance",
    light: "Light",
    dark: "Dark",
    theme_picker_hint: "Use the palette button to choose the theme color.",
    about: "About",
    check_for_updates: "Check for Updates",
    checking_for_updates: "Checking for updates…",
    up_to_date: "MTPDrive is up to date.",
    verifying_update: "Verifying the downloaded DMG…",
    retry_download: "Retry Download",
    show_mtpdrive: "Show MTPDrive",
    rescan_devices: "Rescan Devices",
    quit_mtpdrive: "Quit MTPDrive",
    daemon_program_missing: "The background service could not be found. Reinstall MTPDrive, or build mtpdrive-cli first when running from a development checkout.",
    daemon_start_timeout: "The background service took too long to start.",
    service_hint: "MTPDrive service is not running; start the app or run `mtpdrive daemon`",
    mount_label: "Mount",
    devices_label: "Devices",
    last_error_label: "Last error",
    no_mtp_devices: "No MTP devices connected.",
    serial_label: "serial",
    writable_label: "writable",
    free_label: "free",
    total_label: "total",
    yes: "yes",
    no: "no",
    mtp_device: "MTP device",
    storage: "Storage",
    home_directory_unavailable: "Could not determine the current user’s home directory",
    rename_unsupported: "The device does not support renaming objects",
    move_unsupported: "The device does not support moving objects",
    upload_unsupported: "The device does not support uploads",
    stage_directory_unsupported: "A directory cannot be staged as a file",
    write_too_large: "The write is too large",
    empty_response: "The service returned an empty response",
    io_error: "I/O error",
    mtp_error: "MTP error",
    serialization_error: "Serialization error",
    daemon_unavailable: "The MTPDrive service is not running",
    invalid_response: "Invalid response from the service",
    disconnected: "The device is disconnected",
    not_found: "The object was not found",
    unsupported: "Operation is not supported",
    operation_failed: "Operation failed",
};

const SIMPLIFIED_CHINESE: Strings = Strings {
    devices: "设备",
    logs: "日志",
    settings: "设置",
    settings_description: "设置 MTPDrive 的行为并检查新版本。",
    unmounted: "未挂载",
    mounting: "正在挂载…",
    mount: "挂载",
    unmount: "卸载",
    open_in_finder: "在访达中打开",
    rescan: "重新扫描",
    network_volume: "MTPDrive 网络卷",
    action_required: "需要处理",
    device_action_required: "设备需要处理",
    no_devices: "未发现可用设备",
    connect_android: "用 USB 连接 Android，并在手机上选择“文件传输 / Android Auto”。",
    unknown: "未知",
    read_write: "可读写",
    read_only: "只读",
    clear_view: "清空视图",
    always_open_in_finder: "总是在访达中打开",
    always_open_in_finder_description: "连接第一台 Android 设备时自动打开 MTPDrive 网络卷。",
    open_at_login: "登录时打开",
    open_at_login_description: "登录这台 Mac 时在菜单栏中启动 MTPDrive。",
    open_at_login_requires_approval: "请在系统设置的“登录项”中允许 MTPDrive，以完成启用。",
    open_at_login_unavailable: "只有安装并签名的 MTPDrive 应用才能启用登录时打开。",
    open_login_items_settings: "打开登录项设置",
    language: "语言",
    language_description: "选择应用和后台服务使用的语言。",
    system_default: "跟随系统",
    theme: "主题",
    theme_description: "选择 Material 配色，并设置 MTPDrive 如何跟随系统外观。",
    appearance: "外观",
    light: "浅色",
    dark: "深色",
    theme_picker_hint: "点击调色板按钮选择主题颜色。",
    about: "关于",
    check_for_updates: "检查更新",
    checking_for_updates: "正在检查更新…",
    up_to_date: "MTPDrive 已是最新版本。",
    verifying_update: "正在验证下载的 DMG…",
    retry_download: "重新下载",
    show_mtpdrive: "显示 MTPDrive",
    rescan_devices: "重新扫描设备",
    quit_mtpdrive: "退出 MTPDrive",
    daemon_program_missing: "找不到后台服务程序；请重新安装 MTPDrive，或在开发目录先构建 mtpdrive-cli。",
    daemon_start_timeout: "后台服务启动超时。",
    service_hint: "MTPDrive 服务未运行；请启动应用，或运行 `mtpdrive daemon`",
    mount_label: "挂载状态",
    devices_label: "设备",
    last_error_label: "最近错误",
    no_mtp_devices: "没有已连接的 MTP 设备。",
    serial_label: "序列号",
    writable_label: "可写",
    free_label: "可用",
    total_label: "总计",
    yes: "是",
    no: "否",
    mtp_device: "MTP 设备",
    storage: "存储空间",
    home_directory_unavailable: "无法确定当前用户的个人目录",
    rename_unsupported: "设备不支持重命名对象",
    move_unsupported: "设备不支持移动对象",
    upload_unsupported: "设备不支持上传",
    stage_directory_unsupported: "无法将目录作为文件暂存",
    write_too_large: "写入的数据过大",
    empty_response: "服务返回了空响应",
    io_error: "I/O 错误",
    mtp_error: "MTP 错误",
    serialization_error: "序列化错误",
    daemon_unavailable: "MTPDrive 服务未运行",
    invalid_response: "服务返回了无效响应",
    disconnected: "设备已断开连接",
    not_found: "找不到对象",
    unsupported: "不支持此操作",
    operation_failed: "操作失败",
};

impl Language {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-Hans",
        }
    }

    #[must_use]
    pub const fn strings(self) -> &'static Strings {
        match self {
            Self::English => &ENGLISH,
            Self::SimplifiedChinese => &SIMPLIFIED_CHINESE,
        }
    }

    #[must_use]
    pub fn mounted_at(self, path: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Mounted at {path}"),
            Self::SimplifiedChinese => format!("已挂载到 {path}"),
        }
    }

    #[must_use]
    pub fn mount_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Mount failed: {error}"),
            Self::SimplifiedChinese => format!("挂载失败：{error}"),
        }
    }

    #[must_use]
    pub fn device_count(self, count: usize) -> String {
        match self {
            Self::English if count == 1 => "1 Android MTP device".to_owned(),
            Self::English => format!("{count} Android MTP devices"),
            Self::SimplifiedChinese => format!("{count} 台 Android MTP 设备"),
        }
    }

    #[must_use]
    pub fn device_details(self, serial: &str, usb: &str, writable: bool) -> String {
        let strings = self.strings();
        let access = if writable {
            strings.read_write
        } else {
            strings.read_only
        };
        match self {
            Self::English => format!("Serial {serial}  ·  USB {usb}  ·  {access}"),
            Self::SimplifiedChinese => format!("序列号 {serial}  ·  USB {usb}  ·  {access}"),
        }
    }

    #[must_use]
    pub fn storage_capacity(self, free: &str, total: &str) -> String {
        match self {
            Self::English => format!("{free} available of {total}"),
            Self::SimplifiedChinese => format!("{free} 可用，共 {total}"),
        }
    }

    #[must_use]
    pub fn current_version(self, version: &str) -> String {
        match self {
            Self::English => format!("Current version: {version}"),
            Self::SimplifiedChinese => format!("当前版本：{version}"),
        }
    }

    #[must_use]
    pub fn downloading_update(self, downloaded: u64, total: u64, progress: f32) -> String {
        const MIB: u64 = 1024 * 1024;
        let downloaded_tenths = downloaded.saturating_mul(10) / MIB;
        let total_tenths = total.saturating_mul(10) / MIB;
        let downloaded_mib = format!("{}.{:01}", downloaded_tenths / 10, downloaded_tenths % 10);
        let total_mib = format!("{}.{:01}", total_tenths / 10, total_tenths % 10);
        match self {
            Self::English => format!(
                "Downloading… {:.0}% ({downloaded_mib} of {total_mib} MiB)",
                progress * 100.0
            ),
            Self::SimplifiedChinese => format!(
                "正在下载… {:.0}%（{downloaded_mib} / {total_mib} MiB）",
                progress * 100.0
            ),
        }
    }

    #[must_use]
    pub fn update_downloaded(self, path: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Downloaded and opened {path}"),
            Self::SimplifiedChinese => format!("已下载并打开 {path}"),
        }
    }

    #[must_use]
    pub fn update_download_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not download or open the update: {error}"),
            Self::SimplifiedChinese => format!("无法下载或打开更新：{error}"),
        }
    }

    #[must_use]
    pub fn update_check_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not check for updates: {error}"),
            Self::SimplifiedChinese => format!("检查更新失败：{error}"),
        }
    }

    #[must_use]
    pub fn open_at_login_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not update Open at Login: {error}"),
            Self::SimplifiedChinese => format!("无法更新“登录时打开”：{error}"),
        }
    }

    #[must_use]
    pub fn recent_log_count(self, count: usize) -> String {
        match self {
            Self::English if count == 1 => {
                "The background service’s most recent structured record".to_owned()
            }
            Self::English => {
                format!("The background service’s {count} most recent structured records")
            }
            Self::SimplifiedChinese => format!("后台服务的最近 {count} 条结构化记录"),
        }
    }

    #[must_use]
    pub fn tray_creation_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not create the menu bar icon: {error}"),
            Self::SimplifiedChinese => format!("无法创建菜单栏图标：{error}"),
        }
    }

    #[must_use]
    pub fn invalid_log_response(self, response: impl std::fmt::Debug) -> String {
        match self {
            Self::English => format!("Invalid background log response: {response:?}"),
            Self::SimplifiedChinese => format!("后台日志响应无效：{response:?}"),
        }
    }

    #[must_use]
    pub fn daemon_start_failed(
        self,
        path: impl std::fmt::Display,
        error: impl std::fmt::Display,
    ) -> String {
        match self {
            Self::English => format!("Could not start the background service at {path}: {error}"),
            Self::SimplifiedChinese => format!("无法启动后台服务 {path}：{error}"),
        }
    }

    #[must_use]
    pub fn unexpected_daemon_response(self, response: impl std::fmt::Debug) -> String {
        match self {
            Self::English => format!("Unexpected service response: {response:?}"),
            Self::SimplifiedChinese => format!("服务返回了意外响应：{response:?}"),
        }
    }

    #[must_use]
    pub fn daemon_starting(self, version: &str) -> String {
        match self {
            Self::English => format!("MTPDrive {version} starting"),
            Self::SimplifiedChinese => format!("MTPDrive {version} 正在启动"),
        }
    }

    #[must_use]
    pub fn nfs_listening(self, port: u16) -> String {
        match self {
            Self::English => format!("NFSv3 listening on 127.0.0.1:{port}"),
            Self::SimplifiedChinese => format!("NFSv3 正在监听 127.0.0.1:{port}"),
        }
    }

    #[must_use]
    pub fn nfs_start_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("NFS server did not start: {error}"),
            Self::SimplifiedChinese => format!("NFS 服务未能启动：{error}"),
        }
    }

    #[must_use]
    pub const fn daemon_shutting_down(self) -> &'static str {
        match self {
            Self::English => "MTPDrive shutting down",
            Self::SimplifiedChinese => "MTPDrive 正在退出",
        }
    }

    #[must_use]
    pub fn flush_before_exit_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not upload staged files before exit: {error}"),
            Self::SimplifiedChinese => format!("退出前提交暂存文件失败：{error}"),
        }
    }

    #[must_use]
    pub fn nfs_stopped_with_error(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("NFS server stopped with an error: {error}"),
            Self::SimplifiedChinese => format!("NFS 服务异常停止：{error}"),
        }
    }

    #[must_use]
    pub fn nfs_task_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("NFS server task failed: {error}"),
            Self::SimplifiedChinese => format!("NFS 服务任务失败：{error}"),
        }
    }

    #[must_use]
    pub const fn nfs_shutdown_timeout(self) -> &'static str {
        match self {
            Self::English => "NFS server shutdown timed out",
            Self::SimplifiedChinese => "NFS 服务退出超时",
        }
    }

    #[must_use]
    pub const fn daemon_already_running(self) -> &'static str {
        match self {
            Self::English => "MTPDrive service is already running",
            Self::SimplifiedChinese => "MTPDrive 服务已在运行",
        }
    }

    #[must_use]
    pub fn wait_for_nfs_timeout(self, port: u16) -> String {
        match self {
            Self::English => format!("Timed out waiting for 127.0.0.1:{port}"),
            Self::SimplifiedChinese => format!("等待 127.0.0.1:{port} 超时"),
        }
    }

    #[must_use]
    pub fn device_scan_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Device scan failed: {error}"),
            Self::SimplifiedChinese => format!("扫描设备失败：{error}"),
        }
    }

    #[must_use]
    pub fn device_enumeration_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Device enumeration task failed: {error}"),
            Self::SimplifiedChinese => format!("枚举设备任务失败：{error}"),
        }
    }

    #[must_use]
    pub fn expected_snapshot(self, response: impl std::fmt::Debug) -> String {
        match self {
            Self::English => format!("Expected a status snapshot, got {response:?}"),
            Self::SimplifiedChinese => format!("预期获得状态快照，却收到 {response:?}"),
        }
    }

    #[must_use]
    pub fn reconnect_commit_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not upload staged files after reconnecting: {error}"),
            Self::SimplifiedChinese => format!("重新连接后提交暂存文件失败：{error}"),
        }
    }

    #[must_use]
    pub fn device_disconnected(self, key: &str) -> String {
        match self {
            Self::English => format!("Device disconnected: {key}"),
            Self::SimplifiedChinese => format!("设备已断开连接：{key}"),
        }
    }

    #[must_use]
    pub fn device_connected(self, manufacturer: &str, model: &str) -> String {
        match self {
            Self::English => format!("Connected to {manufacturer} {model}"),
            Self::SimplifiedChinese => format!("已连接到 {manufacturer} {model}"),
        }
    }

    #[must_use]
    pub fn image_capture_reclaimed(self, count: usize) -> String {
        match self {
            Self::English => format!(
                "Stopped {count} macOS Image Capture process(es); claiming the MTP interface"
            ),
            Self::SimplifiedChinese => {
                format!("已结束 {count} 个 macOS 图像捕捉进程，正在接管 MTP 接口")
            }
        }
    }

    #[must_use]
    pub fn image_capture_reclaim_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not stop macOS Image Capture: {error}"),
            Self::SimplifiedChinese => format!("无法结束 macOS 图像捕捉进程：{error}"),
        }
    }

    #[must_use]
    pub fn storage_refresh_failed(self, key: &str, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not refresh storage for {key}: {error}"),
            Self::SimplifiedChinese => format!("刷新 {key} 的存储信息失败：{error}"),
        }
    }

    #[must_use]
    pub fn open_device_failed(self, key: &str, detail: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Could not open Android device {key}: {detail}"),
            Self::SimplifiedChinese => format!("无法打开 Android 设备 {key}：{detail}"),
        }
    }

    #[must_use]
    pub fn device_exclusively_held(self, key: &str) -> String {
        match self {
            Self::English => format!(
                "Could not open Android device {key} because another app has exclusive access. Close Preview, Photos, Image Capture, or any other app using the phone; reconnect USB; then select “File transfer / Android Auto” on the phone."
            ),
            Self::SimplifiedChinese => format!(
                "无法打开 Android 设备 {key}：它正被另一个程序独占。请关闭“预览”、“照片”或“图像捕捉”等正在访问手机的程序，重新连接 USB，并在手机上选择“文件传输 / Android Auto”。"
            ),
        }
    }

    #[must_use]
    pub const fn committing_staged_file(self) -> &'static str {
        match self {
            Self::English => "Committing staged file",
            Self::SimplifiedChinese => "正在提交暂存文件",
        }
    }

    #[must_use]
    pub fn background_commit_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Background upload of a staged file failed: {error}"),
            Self::SimplifiedChinese => format!("后台提交暂存文件失败：{error}"),
        }
    }

    #[must_use]
    pub fn uploaded(self, name: &str) -> String {
        match self {
            Self::English => format!("Uploaded {name}"),
            Self::SimplifiedChinese => format!("已上传 {name}"),
        }
    }

    #[must_use]
    pub fn mounted(self, path: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Mounted {path}"),
            Self::SimplifiedChinese => format!("已挂载 {path}"),
        }
    }

    #[must_use]
    pub const fn unmounted_log(self) -> &'static str {
        match self {
            Self::English => "Unmounted MTPDrive",
            Self::SimplifiedChinese => "已卸载 MTPDrive",
        }
    }

    #[must_use]
    pub const fn replacing_stale_mount(self) -> &'static str {
        match self {
            Self::English => "Replacing a stale MTPDrive mount from an earlier service",
            Self::SimplifiedChinese => "正在替换旧服务遗留的失效 MTPDrive 挂载",
        }
    }

    #[must_use]
    pub fn mount_command_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("mount_nfs failed: {error}"),
            Self::SimplifiedChinese => format!("mount_nfs 执行失败：{error}"),
        }
    }

    #[must_use]
    pub fn unmount_failed(self, error: impl std::fmt::Display) -> String {
        match self {
            Self::English => format!("Unmount failed: {error}"),
            Self::SimplifiedChinese => format!("卸载失败：{error}"),
        }
    }

    #[must_use]
    pub const fn finder_open_failed(self) -> &'static str {
        match self {
            Self::English => "Could not open Finder",
            Self::SimplifiedChinese => "无法打开访达",
        }
    }
}

/// Returns the process-wide language currently selected by the user.
#[must_use]
pub fn current_language() -> Language {
    match CURRENT_LANGUAGE.load(Ordering::Acquire) {
        1 => Language::English,
        2 => Language::SimplifiedChinese,
        _ => system_language(),
    }
}

/// Updates the process-wide language used for new UI and service messages.
pub fn set_current_language(language: Language) {
    CURRENT_LANGUAGE.store(
        match language {
            Language::English => 1,
            Language::SimplifiedChinese => 2,
        },
        Ordering::Release,
    );
}

/// Returns the language detected from macOS and locale environment settings.
#[must_use]
pub fn system_language() -> Language {
    *SYSTEM_LANGUAGE.get_or_init(detect_language)
}

/// Parses a BCP-47 or POSIX locale identifier supported by `MTPDrive`.
#[must_use]
pub fn parse_language(value: &str) -> Option<Language> {
    let normalized = value
        .trim()
        .trim_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, '"' | '\'' | ',' | '(' | ')')
        })
        .split('.')
        .next()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_lowercase();
    let primary = normalized.split('-').next().unwrap_or_default();
    match primary {
        "en" => Some(Language::English),
        "zh" if !normalized.starts_with("zh-hant")
            && !normalized.starts_with("zh-tw")
            && !normalized.starts_with("zh-hk")
            && !normalized.starts_with("zh-mo") =>
        {
            Some(Language::SimplifiedChinese)
        }
        _ => None,
    }
}

/// Selects the first supported locale, falling back to English.
#[must_use]
pub fn detect_language_from(values: &[&str]) -> Language {
    values
        .iter()
        .find_map(|value| parse_language_list(value))
        .unwrap_or_default()
}

fn detect_language() -> Language {
    if let Ok(language) = std::env::var("MTPDRIVE_LANG")
        && let Some(language) = parse_language_list(&language)
    {
        return language;
    }

    #[cfg(target_os = "macos")]
    {
        for arguments in [
            ["read", APP_DEFAULTS_DOMAIN, "AppleLanguages"],
            ["read", "-g", "AppleLanguages"],
        ] {
            if let Ok(output) = Command::new("/usr/bin/defaults").args(arguments).output()
                && output.status.success()
                && let Some(language) =
                    parse_language_list(&String::from_utf8_lossy(&output.stdout))
            {
                return language;
            }
        }
    }

    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(variable)
            && let Some(language) = parse_language_list(&value)
        {
            return language;
        }
    }
    Language::default()
}

fn parse_language_list(value: &str) -> Option<Language> {
    value
        .split(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ',' | '(' | ')' | '"' | '\'')
        })
        .filter(|candidate| !candidate.is_empty())
        .find_map(parse_language)
}

#[cfg(test)]
#[path = "../tests/unit/i18n.rs"]
mod tests;
