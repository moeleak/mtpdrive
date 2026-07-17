mod devices;
mod logs;
mod settings;

use crate::application::{App, LanguageOption, Message, Page};
use material::widget::navigation;
use material_ui_rs as material;
use mtpdrive_core::{Language, LanguagePreference};

pub(crate) fn view(app: &App) -> material::Element<'_, Message> {
    let page_content = match app.navigation.selected() {
        Page::Devices => devices::view(app),
        Page::Logs => logs::view(app),
        Page::Settings => settings::view(app),
    };
    navigation::suite(&app.destinations, &app.navigation)
        .layout(navigation::adaptive_layout(
            app.window_size.width,
            app.window_size.height,
        ))
        .with_menu("MTPDrive", Message::MenuPressed)
        .view(Message::Navigate, page_content)
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
