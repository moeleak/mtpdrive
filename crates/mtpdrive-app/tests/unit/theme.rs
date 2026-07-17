use super::{effective_dark, material_color, persisted_color};
use iced::theme::Mode;
use material::widget::theme_picker::MaterialColor;
use material_ui_rs as material;
use mtpdrive_core::{AppearancePreference, ThemeColor};

#[test]
fn appearance_preference_resolves_against_system_mode() {
    assert!(!effective_dark(AppearancePreference::System, Mode::Light));
    assert!(effective_dark(AppearancePreference::System, Mode::Dark));
    assert!(!effective_dark(AppearancePreference::Light, Mode::Dark));
    assert!(effective_dark(AppearancePreference::Dark, Mode::Light));
}

#[test]
fn persisted_colors_round_trip_through_material_colors() {
    let colors = [
        ThemeColor::Purple,
        ThemeColor::Blue,
        ThemeColor::Teal,
        ThemeColor::Green,
        ThemeColor::Yellow,
        ThemeColor::Orange,
        ThemeColor::Red,
        ThemeColor::Pink,
    ];

    for color in colors {
        assert_eq!(persisted_color(material_color(color)), color);
    }
    assert_eq!(material_color(ThemeColor::Blue), MaterialColor::Blue);
}
