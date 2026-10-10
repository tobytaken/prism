use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Apt,
    Dnf,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SecureBootState {
    Enabled,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriverHostInfo {
    pub distro_id: String,
    pub distro_version: String,
    pub kernel_release: String,
    pub package_manager: PackageManager,
    pub secure_boot: SecureBootState,
    pub is_flatpak: bool,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

pub fn collect_host_info() -> DriverHostInfo {
    let os_release = parse_os_release_path(Path::new("/etc/os-release"));
    let distro_id = os_release
        .get("ID")
        .cloned()
        .unwrap_or_else(|| "unknown".into())
        .to_lowercase();
    let distro_version = os_release
        .get("VERSION_ID")
        .cloned()
        .unwrap_or_else(|| "unknown".into());
    let package_manager = package_manager_for_distro(&distro_id);
    let unsupported_reason = match package_manager {
        PackageManager::Apt if !command_exists("apt-get") => Some("apt-get was not found".into()),
        PackageManager::Apt if !command_exists("ubuntu-drivers") && distro_id == "ubuntu" => {
            Some("ubuntu-drivers was not found".into())
        }
        PackageManager::Dnf if !command_exists("dnf") => Some("dnf was not found".into()),
        PackageManager::Unsupported => Some(format!(
            "Automated driver management is not supported on {distro_id}"
        )),
        _ => None,
    };

    DriverHostInfo {
        distro_id,
        distro_version,
        kernel_release: kernel_release(),
        package_manager,
        secure_boot: detect_secure_boot(Path::new("/sys/firmware/efi/efivars")),
        is_flatpak: env::var_os("FLATPAK_ID").is_some(),
        supported: false,
        unsupported_reason,
    }
}

pub fn parse_os_release_path(path: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(path)
        .map(|contents| parse_os_release(&contents))
        .unwrap_or_default()
}

pub fn parse_os_release(contents: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(key.to_string(), unquote_os_release_value(value));
    }

    values
}

pub fn detect_secure_boot(efivars_path: &Path) -> SecureBootState {
    let entries = match fs::read_dir(efivars_path) {
        Ok(entries) => entries,
        Err(_) => return SecureBootState::Unknown,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("SecureBoot-") {
            continue;
        }

        let Ok(bytes) = fs::read(entry.path()) else {
            return SecureBootState::Unknown;
        };
        let Some(value) = bytes.get(4) else {
            return SecureBootState::Unknown;
        };

        return if *value == 1 {
            SecureBootState::Enabled
        } else {
            SecureBootState::Disabled
        };
    }

    SecureBootState::Unknown
}

pub fn command_exists(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| {
        let candidate = path.join(command);
        candidate.is_file()
    })
}

fn package_manager_for_distro(distro_id: &str) -> PackageManager {
    match distro_id {
        "ubuntu" => PackageManager::Apt,
        "fedora" => PackageManager::Dnf,
        _ => PackageManager::Unsupported,
    }
}

fn kernel_release() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn unquote_os_release_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{SecureBootState, detect_secure_boot, parse_os_release};
    use std::fs;

    #[test]
    fn parses_quoted_os_release_values() {
        let parsed = parse_os_release("ID=ubuntu\nVERSION_ID=\"24.04\"\nNAME=\"Ubuntu Linux\"\n");

        assert_eq!(parsed["ID"], "ubuntu");
        assert_eq!(parsed["VERSION_ID"], "24.04");
        assert_eq!(parsed["NAME"], "Ubuntu Linux");
    }

    #[test]
    fn parses_secure_boot_enabled_efivar() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path()
                .join("SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            [0, 0, 0, 0, 1],
        )
        .expect("write");

        assert_eq!(detect_secure_boot(dir.path()), SecureBootState::Enabled);
    }

    #[test]
    fn parses_secure_boot_disabled_efivar() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path()
                .join("SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            [0, 0, 0, 0, 0],
        )
        .expect("write");

        assert_eq!(detect_secure_boot(dir.path()), SecureBootState::Disabled);
    }
}
