//! What the host says about prismd apart from its socket, and reading it from what systemctl and
//! os-release print. Asking is the caller's: this only parses.

/// What the host says about prismd, apart from its socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrismdHost {
    pub install: PrismdInstall,
    /// The host's distribution, which decides the commands that install prismd.
    pub distro: Distro,
}

/// How Prism is installed, which decides what it can find out about prismd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrismdInstall {
    /// Prism runs on the host, and systemd says how prismd.service is.
    Native(UnitState),
    /// Prism runs in a Flatpak sandbox, which can't talk to systemd. It sees `/run/prism` only if
    /// prismd was installed before the sandbox started.
    Flatpak { run_dir_visible: bool },
}

/// prismd.service as `systemctl show` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitState {
    /// systemd has no prismd.service: prismd isn't installed.
    NotFound,
    /// Installed, but not running: not enabled, or enabled and stopped.
    Inactive { enabled: bool },
    /// Running, or starting.
    Active,
    /// systemd gave up on it, with its `Result`, such as "exit-code" or "watchdog".
    Failed { result: String },
    /// systemctl couldn't say, as `message` says.
    Unknown { message: String },
}

/// The host's distribution, from its os-release file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Distro {
    /// Such as "ubuntu" or "fedora".
    pub id: String,
    /// The distributions it derives from, such as "debian" for Ubuntu.
    pub id_like: Vec<String>,
    /// Its pretty name, such as "Ubuntu 24.04 LTS".
    pub name: String,
}

impl UnitState {
    /// Reads `systemctl show prismd.service
    /// --property=LoadState,ActiveState,Result,UnitFileState`.
    pub fn from_systemctl_show(output: &str) -> Self {
        let properties = properties(output);
        let property = |name| property(&properties, name);
        if property("LoadState") == Some("not-found") {
            return Self::NotFound;
        }
        match property("ActiveState") {
            Some("active" | "activating" | "reloading" | "refreshing") => Self::Active,
            Some("inactive" | "deactivating" | "maintenance") => Self::Inactive {
                // Also "enabled-runtime", enabled until the next boot.
                enabled: property("UnitFileState")
                    .is_some_and(|state| state.starts_with("enabled")),
            },
            Some("failed") => Self::Failed {
                result: property("Result").unwrap_or_default().to_owned(),
            },
            Some(other) => Self::Unknown {
                message: format!("systemctl says prismd.service is {other}"),
            },
            None => Self::Unknown {
                message: "systemctl didn't say how prismd.service is".into(),
            },
        }
    }
}

impl Distro {
    /// Reads an os-release file, with its defaults for what it leaves out.
    pub fn from_os_release(contents: &str) -> Self {
        let properties = properties(contents);
        let property = |name| {
            property(&properties, name)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self {
            id: property("ID").unwrap_or_else(|| "linux".into()),
            id_like: property("ID_LIKE")
                .map(|like| like.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
            name: property("PRETTY_NAME")
                .or_else(|| property("NAME"))
                .unwrap_or_else(|| "Linux".into()),
        }
    }
}

/// The value of the property `name` among `properties`, if it's there.
fn property<'a>(properties: &'a [(&str, String)], name: &str) -> Option<&'a str> {
    properties
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_str())
}

/// The `KEY=value` lines of `text`, with any quotes around a value taken off and comments left
/// out.
fn properties(text: &str) -> Vec<(&str, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let unquoted = ['"', '\'']
                .into_iter()
                .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
                .unwrap_or(value);
            (key.trim(), unquoted.to_owned())
        })
        .collect()
}
