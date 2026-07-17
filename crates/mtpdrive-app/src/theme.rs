use iced::theme::Mode;
use material::widget::theme_picker::MaterialColor;
use material_ui_rs as material;
use mtpdrive_core::{AppearancePreference, ThemeColor};

pub(crate) const fn effective_dark(preference: AppearancePreference, system_mode: Mode) -> bool {
    match preference {
        AppearancePreference::System => !matches!(system_mode, Mode::Light),
        AppearancePreference::Light => false,
        AppearancePreference::Dark => true,
    }
}

pub(crate) const fn material_color(color: ThemeColor) -> MaterialColor {
    match color {
        ThemeColor::Purple => MaterialColor::Purple,
        ThemeColor::Blue => MaterialColor::Blue,
        ThemeColor::Teal => MaterialColor::Teal,
        ThemeColor::Green => MaterialColor::Green,
        ThemeColor::Yellow => MaterialColor::Yellow,
        ThemeColor::Orange => MaterialColor::Orange,
        ThemeColor::Red => MaterialColor::Red,
        ThemeColor::Pink => MaterialColor::Pink,
    }
}

pub(crate) const fn persisted_color(color: MaterialColor) -> ThemeColor {
    match color {
        MaterialColor::Purple => ThemeColor::Purple,
        MaterialColor::Blue => ThemeColor::Blue,
        MaterialColor::Teal => ThemeColor::Teal,
        MaterialColor::Green => ThemeColor::Green,
        MaterialColor::Yellow => ThemeColor::Yellow,
        MaterialColor::Orange => ThemeColor::Orange,
        MaterialColor::Red => ThemeColor::Red,
        MaterialColor::Pink => ThemeColor::Pink,
    }
}

#[cfg(test)]
#[path = "../tests/unit/theme.rs"]
mod tests;
