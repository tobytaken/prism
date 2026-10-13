//! The GUI's own settings, kept per user in a file under the XDG config directory.

use std::path::{Path, PathBuf};

use prism_ui::{AppearanceMode, GuiSettings};
use serde::{Deserialize, Serialize};

/// Where the GUI's settings are kept.
pub trait SettingsStore {
    /// The saved settings, or the defaults when there are none or they can't be read.
    fn load(&self) -> GuiSettings;

    /// Saves `settings` in place of the saved ones, or says why it can't on stderr.
    fn save(&self, settings: &GuiSettings);
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
            Ok(contents) => contents.into(),
            Err(error) => {
                eprintln!("prism: ignoring unreadable {}: {error}", path.display());
                GuiSettings::default()
            }
        }
    }

    fn save(&self, settings: &GuiSettings) {
        let Some(path) = &self.path else {
            eprintln!("prism: can't save settings without a config directory");
            return;
        };
        if let Err(error) = write(path, &SettingsContents::from(*settings)) {
            eprintln!("prism: can't save {}: {error}", path.display());
        }
    }
}

/// Writes `contents` to `path` whole: through a file beside it that takes its place, so a crash
/// midway leaves the old settings, not half of the new ones.
fn write(path: &Path, contents: &SettingsContents) -> std::io::Result<()> {
    let text = toml::to_string(contents).map_err(std::io::Error::other)?;
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

#[derive(Debug, Deserialize, Serialize)]
struct SettingsContents {
    #[serde(default)]
    appearance: Appearance,
    #[serde(default = "on")]
    desktop_notifications: bool,
    #[serde(default)]
    compact_mode: bool,
}

fn on() -> bool {
    true
}

impl From<SettingsContents> for GuiSettings {
    fn from(contents: SettingsContents) -> Self {
        Self {
            appearance: contents.appearance.into(),
            desktop_notifications: contents.desktop_notifications,
            compact_mode: contents.compact_mode,
        }
    }
}

impl From<GuiSettings> for SettingsContents {
    fn from(settings: GuiSettings) -> Self {
        Self {
            appearance: settings.appearance.into(),
            desktop_notifications: settings.desktop_notifications,
            compact_mode: settings.compact_mode,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
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

impl From<AppearanceMode> for Appearance {
    fn from(appearance: AppearanceMode) -> Self {
        match appearance {
            AppearanceMode::System => Self::System,
            AppearanceMode::Light => Self::Light,
            AppearanceMode::Dark => Self::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use prism_ui::{AppearanceMode, GuiSettings};

    use super::{SettingsFile, SettingsStore};

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

    #[test]
    fn the_settings_file_holds_whether_desktop_notifications_are_on() {
        assert!(settings_from(Some("appearance = \"dark\"\n")).desktop_notifications);
        assert!(!settings_from(Some("desktop_notifications = false\n")).desktop_notifications);
    }

    #[test]
    fn the_settings_file_holds_whether_compact_mode_is_on() {
        assert!(!settings_from(Some("appearance = \"dark\"\n")).compact_mode);
        assert!(settings_from(Some("compact_mode = true\n")).compact_mode);
    }

    #[test]
    fn saved_settings_load_again_from_a_new_settings_directory() {
        let directory = tempfile::tempdir().expect("create a temporary directory");
        let file = SettingsFile::at(directory.path().join("prism").join("settings.toml"));
        let settings = GuiSettings {
            appearance: AppearanceMode::Light,
            desktop_notifications: false,
            compact_mode: true,
        };

        file.save(&settings);

        assert_eq!(file.load(), settings);
        let saved = GuiSettings {
            appearance: AppearanceMode::System,
            ..settings
        };
        file.save(&saved);
        assert_eq!(file.load(), saved);
    }
}
