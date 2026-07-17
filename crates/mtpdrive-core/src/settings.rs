use crate::i18n::{Language, system_language};
use crate::{AppPaths, Result};
use serde::{Deserialize, Serialize};

/// Language selection persisted independently from the detected system locale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguagePreference {
    #[default]
    System,
    English,
    SimplifiedChinese,
}

impl LanguagePreference {
    #[must_use]
    pub fn resolve(self) -> Language {
        match self {
            Self::System => system_language(),
            Self::English => Language::English,
            Self::SimplifiedChinese => Language::SimplifiedChinese,
        }
    }
}

/// Material color theme persisted without coupling the core crate to the UI toolkit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeColor {
    #[default]
    Purple,
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
}

/// Appearance selection persisted independently from the current system theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearancePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// User preferences shared by the app and background service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub always_open_in_finder: bool,
    pub language: LanguagePreference,
    pub theme_color: ThemeColor,
    pub appearance: AppearancePreference,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            always_open_in_finder: true,
            language: LanguagePreference::System,
            theme_color: ThemeColor::Purple,
            appearance: AppearancePreference::System,
        }
    }
}

impl AppSettings {
    /// Loads preferences, returning defaults when no preferences were saved yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the settings file cannot be read or decoded.
    pub fn load(paths: &AppPaths) -> Result<Self> {
        match std::fs::read(&paths.settings_path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Atomically saves preferences in the per-user application support directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the support directory or settings file cannot be written.
    pub fn save(self, paths: &AppPaths) -> Result<()> {
        paths.ensure()?;
        let bytes = serde_json::to_vec_pretty(&self)?;
        let temporary = paths
            .settings_path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&temporary, bytes)?;
        if let Err(error) = std::fs::rename(&temporary, &paths.settings_path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error.into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/settings.rs"]
mod tests;
