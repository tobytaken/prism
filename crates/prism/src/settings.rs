//! The GUI's own settings, kept per user in a file under the XDG config directory.

use std::path::PathBuf;

use prism_ui::AppearanceMode;
use serde::Deserialize;

/// The settings the GUI keeps for itself, apart from prismd's tuning store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuiSettings {
    pub appearance: AppearanceMode,
}

/// Where the GUI's settings are kept.
pub trait SettingsStore {
    /// The saved settings, or the defaults when there are none or they can't be read.
    fn load(&self) -> GuiSettings;
}

/// Settings in a TOML file, by default `$XDG_CONFIG_HOME/prism/settings.toml`.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: Option<PathBuf>,
}

impl SettingsFile {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// The user's settings file, under `$XDG_CONFIG_HOME`, or `~/.config` when that is unset.
    pub fn in_user_config() -> Self {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|directory| !directory.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        Self {
            path: config.map(|directory| directory.join("prism").join("settings.toml")),
        }
    }
}

impl SettingsStore for SettingsFile {
    fn load(&self) -> GuiSettings {
        let Some(path) = &self.path else {
            return GuiSettings::default();
        };
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return GuiSettings::default();
            }
            Err(error) => {
                eprintln!("prism: can't read {}: {error}", path.display());
                return GuiSettings::default();
            }
        };
        match toml::from_str::<SettingsContents>(&contents) {
            Ok(contents) => GuiSettings {
                appearance: contents.appearance.into(),
            },
            Err(error) => {
                eprintln!("prism: ignoring unreadable {}: {error}", path.display());
                GuiSettings::default()
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct SettingsContents {
    #[serde(default)]
    appearance: Appearance,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl From<Appearance> for AppearanceMode {
    fn from(appearance: Appearance) -> Self {
        match appearance {
            Appearance::System => Self::System,
            Appearance::Light => Self::Light,
            Appearance::Dark => Self::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use prism_ui::AppearanceMode;

    use super::{GuiSettings, SettingsFile, SettingsStore};

    fn settings_from(contents: Option<&str>) -> GuiSettings {
        let directory = tempfile::tempdir().expect("create a temporary directory");
        let path = directory.path().join("prism").join("settings.toml");
        if let Some(contents) = contents {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
        }
        SettingsFile::at(path).load()
    }

    #[test]
    fn a_missing_settings_file_gives_the_defaults() {
        assert_eq!(settings_from(None), GuiSettings::default());
        assert_eq!(GuiSettings::default().appearance, AppearanceMode::System);
    }

    #[test]
    fn the_settings_file_holds_the_appearance_mode() {
        assert_eq!(
            settings_from(Some("appearance = \"dark\"\n")).appearance,
            AppearanceMode::Dark
        );
        assert_eq!(
            settings_from(Some("appearance = \"light\"\n")).appearance,
            AppearanceMode::Light
        );
    }

    #[test]
    fn an_unreadable_settings_file_gives_the_defaults() {
        assert_eq!(
            settings_from(Some("appearance = \"sepia\"\n")),
            GuiSettings::default()
        );
        assert_eq!(settings_from(Some("not toml")), GuiSettings::default());
    }
}
