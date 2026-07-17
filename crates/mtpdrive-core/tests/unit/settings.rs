use super::{AppSettings, LanguagePreference};
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
    };

    expected.save(&paths).expect("save settings");

    assert_eq!(AppSettings::load(&paths).expect("load settings"), expected);
}
