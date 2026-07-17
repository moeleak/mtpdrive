mod devices;
mod logs;
mod settings;

use crate::application::{App, AppearanceOption, LanguageOption, Message, Page};
use iced::time::Instant;
use material::widget::{navigation, theme_picker};
use material_ui_rs as material;
use mtpdrive_core::{AppearancePreference, Language, LanguagePreference};

pub(crate) fn view(app: &App) -> material::Element<'_, Message> {
    let page_content = match app.navigation.selected() {
        Page::Devices => devices::view(app),
        Page::Logs => logs::view(app),
        Page::Settings => settings::view(app),
    };
    let layout = navigation::adaptive_layout(app.window_size.width, app.window_size.height);
    let content = navigation::suite(&app.destinations, &app.navigation)
        .layout(layout)
        .with_menu("MTPDrive", Message::MenuPressed)
        .view(Message::Navigate, page_content);
    let content = if app.navigation.selected() == Page::Settings {
        app.theme_controller.controls_over(
            content,
            theme_picker::bottom_margin(layout),
            Message::ThemeChanged,
        )
    } else {
        content
    };
    app.theme_controller.reveal_over(content, Instant::now())
}

pub(crate) fn appearance_options(language: Language) -> [AppearanceOption; 3] {
    let strings = language.strings();
    [
        AppearanceOption {
            preference: AppearancePreference::System,
            label: strings.system_default,
        },
        AppearanceOption {
            preference: AppearancePreference::Light,
            label: strings.light,
        },
        AppearanceOption {
            preference: AppearancePreference::Dark,
            label: strings.dark,
        },
    ]
}

pub(crate) fn destinations(language: Language) -> [navigation::Destination<Page>; 3] {
    let strings = language.strings();
    [
        navigation::Destination::new(Page::Devices, "devices", strings.devices),
        navigation::Destination::new(Page::Logs, "description", strings.logs),
        navigation::Destination::new(Page::Settings, "settings", strings.settings),
    ]
}

pub(crate) fn language_options(language: Language) -> [LanguageOption; 3] {
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

#[allow(clippy::cast_precision_loss)]
pub(super) fn progress_ratio(value: u64, total: u64) -> f32 {
    value as f32 / total as f32
}
