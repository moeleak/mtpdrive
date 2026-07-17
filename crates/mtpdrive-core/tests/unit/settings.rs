use super::{AppSettings, AppearancePreference, LanguagePreference, ThemeColor};
use crate::AppPaths;

#[test]
fn missing_settings_use_defaults() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let paths = AppPaths::under(directory.path());

    assert_eq!(
        AppSettings::load(&paths).expect("default settings"),
        AppSettings::default()
    );
}

#[test]
fn settings_round_trip() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let paths = AppPaths::under(directory.path());
    let expected = AppSettings {
        always_open_in_finder: false,
        language: LanguagePreference::SimplifiedChinese,
        theme_color: ThemeColor::Teal,
        appearance: AppearancePreference::Dark,
    };

    expected.save(&paths).expect("save settings");

    assert_eq!(AppSettings::load(&paths).expect("load settings"), expected);
}

#[test]
fn legacy_settings_use_theme_defaults() {
    let settings: AppSettings =
        serde_json::from_str(r#"{"always_open_in_finder":false,"language":"english"}"#)
            .expect("legacy settings");

    assert_eq!(settings.theme_color, ThemeColor::Purple);
    assert_eq!(settings.appearance, AppearancePreference::System);
}
