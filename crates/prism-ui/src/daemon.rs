//! Settings → Daemon: why there's no tuning access and how to fix it, by the prismd connection's
//! reason, and the commands that set prismd up on the host.

use prism_protocol::{Authorization, ProtocolVersion, Welcome};

use crate::core::{Core, NotConnected, PrismdConnection, Session};
use crate::host::{Distro, PrismdHost, PrismdInstall, UnitState};

/// What Settings → Daemon shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonSettings {
    /// What the connection is, such as "prismd isn't running".
    pub title: String,
    /// What it means and what to do, a paragraph each.
    pub paragraphs: Vec<String>,
    pub facts: Vec<Fact>,
    /// Commands to copy and run on the host, in order.
    pub commands: Vec<String>,
    /// What Prism can do about it itself, natively.
    pub action: Option<DaemonAction>,
    /// Why the last Enable or Restart failed.
    pub error: Option<String>,
}

/// A labeled fact, such as prismd's socket path or a GPU's driver version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

/// The button that runs a service action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonAction {
    pub action: ServiceAction,
    /// Such as "Enable prismd", or "Enabling prismd…" while it runs.
    pub title: String,
    /// Whether it's running, so it can't be pressed again.
    pub busy: bool,
}

/// What Prism can ask systemd to do with prismd.service, under polkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    /// `systemctl enable --now prismd.service`.
    Enable,
    /// `systemctl restart prismd.service`.
    Restart,
}

impl ServiceAction {
    pub fn title(self) -> &'static str {
        match self {
            Self::Enable => "Enable prismd",
            Self::Restart => "Restart prismd",
        }
    }

    pub(crate) fn running(self) -> &'static str {
        match self {
            Self::Enable => "Enabling prismd…",
            Self::Restart => "Restarting prismd…",
        }
    }

    /// Why it failed, as `message` from systemctl says, for Settings → Daemon.
    pub(crate) fn failure(self, message: &str) -> String {
        let verb = match self {
            Self::Enable => "enable",
            Self::Restart => "restart",
        };
        format!("Couldn't {verb} prismd: {message}")
    }
}

/// How a service action ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceOutcome {
    Done,
    /// The user canceled polkit's prompt, which leaves everything as it was.
    Canceled,
    Failed {
        message: String,
    },
}

const ENABLE: &str = "sudo systemctl enable --now prismd";
// Resetting first also clears systemd's start limit, which a plain restart can't get past.
const RESTART: &str = "sudo systemctl reset-failed prismd && sudo systemctl restart prismd";
const JOURNAL: &str = "journalctl -u prismd -b";
const WHY_PRISMD: &str = "Tuning, fan curves and game switching need prismd, Prism's host daemon. \
                          Monitoring works without it.";

impl Core {
    pub fn daemon_settings(&self) -> DaemonSettings {
        let mut settings = DaemonSettings {
            title: String::new(),
            paragraphs: Vec::new(),
            facts: Vec::new(),
            commands: Vec::new(),
            action: None,
            error: self.service_error.clone(),
        };
        let flatpak = matches!(
            self.host.as_ref().map(|host| &host.install),
            Some(PrismdInstall::Flatpak { .. })
        );
        let text = |lines: &[&str]| lines.iter().map(|&line| line.to_owned()).collect();
        match self.prismd() {
            PrismdConnection::Connecting => settings.title = "Connecting to prismd…".into(),
            PrismdConnection::NotConnected(NotConnected::NotInstalled) => {
                settings.title = "prismd isn't installed".into();
                let distro = self
                    .host
                    .as_ref()
                    .map(|host| host.distro.clone())
                    .unwrap_or_default();
                settings.paragraphs.push(WHY_PRISMD.into());
                match install_command(&distro) {
                    Some(install) => {
                        let source = match install {
                            AUR_INSTALL => "from the AUR, such as with yay".to_owned(),
                            _ => format!("on {}", distro.name),
                        };
                        settings
                            .paragraphs
                            .push(format!("Install it {source}, then start it:"));
                        settings.commands = vec![install.into(), ENABLE.into()];
                    }
                    None => {
                        settings.paragraphs.push(format!(
                            "Install the prismd package for {}, then start it:",
                            distro.name
                        ));
                        settings.commands = vec![ENABLE.into()];
                    }
                }
                if flatpak {
                    // The sandbox sees /run/prism only if it was there when Prism started.
                    settings
                        .paragraphs
                        .push("Just installed it? Restart Prism.".into());
                }
            }
            PrismdConnection::NotConnected(NotConnected::NotRunning) => {
                settings.title = "prismd isn't running".into();
                let unit = match self.host.as_ref().map(|host| &host.install) {
                    Some(PrismdInstall::Native(unit)) => Some(unit),
                    _ => None,
                };
                settings.paragraphs = match unit {
                    Some(UnitState::Inactive { enabled: true }) => {
                        text(&["prismd is enabled, but has stopped."])
                    }
                    Some(UnitState::Inactive { enabled: false }) => {
                        text(&["prismd is installed, but isn't enabled."])
                    }
                    // As right after Enable, until the next attempt to connect.
                    Some(UnitState::Active) => text(&[
                        "systemd says prismd is running, but it isn't answering yet. Prism \
                         connects as soon as it does.",
                    ]),
                    _ if flatpak => {
                        settings.commands = text(&[ENABLE]);
                        text(&[
                            "prismd is installed, but isn't enabled or has stopped. Start it on \
                             the host:",
                        ])
                    }
                    _ => text(&["prismd is installed, but isn't enabled or has stopped."]),
                };
            }
            PrismdConnection::NotConnected(NotConnected::Failed { result }) => {
                settings.title = "prismd failed".into();
                let gave_up = match explain_result(&result) {
                    Some(why) => format!("systemd gave up on it with result {result}: {why}."),
                    None => format!("systemd gave up on it with result {result}."),
                };
                settings.paragraphs = vec![gave_up, "Its log says why:".into()];
                settings.commands = text(&[JOURNAL]);
            }
            PrismdConnection::NotConnected(NotConnected::Incompatible(incompatible)) => {
                settings.title = "prismd speaks another protocol version".into();
                let update = if incompatible.protocol > ProtocolVersion::CURRENT {
                    "prismd is newer than Prism: update Prism."
                } else {
                    "prismd is older than Prism: update prismd."
                };
                settings.paragraphs = text(&[update]);
                settings.facts = vec![
                    fact(
                        "Prism version",
                        format!(
                            "{} · protocol {}",
                            env!("CARGO_PKG_VERSION"),
                            ProtocolVersion::CURRENT
                        ),
                    ),
                    fact(
                        "prismd version",
                        format!(
                            "{} · protocol {}",
                            incompatible.prismd_version, incompatible.protocol
                        ),
                    ),
                ];
            }
            PrismdConnection::Connected(welcome) => {
                self.connected_settings(&welcome, &mut settings)
            }
        }
        settings.action = self.service_action().map(|action| {
            let busy = self.service == Some(action);
            DaemonAction {
                action,
                title: if busy {
                    action.running()
                } else {
                    action.title()
                }
                .into(),
                busy,
            }
        });
        settings
    }

    fn connected_settings(&self, welcome: &Welcome, settings: &mut DaemonSettings) {
        let reason = welcome
            .reason
            .as_deref()
            .unwrap_or("prismd doesn't allow it");
        settings.title = if welcome.can_tune {
            "Connected to prismd"
        } else {
            "No tuning access"
        }
        .into();
        settings.facts.push(fact(
            "Tuning access",
            if welcome.can_tune {
                "Yes".into()
            } else {
                format!("No: {reason}")
            },
        ));
        match &welcome.authorization {
            Some(Authorization::NotInAdminGroup { user, group }) => {
                settings.paragraphs = vec![
                    format!(
                        "User {user} isn't in group {group}, so prismd lets this Prism monitor \
                         GPUs but not tune them."
                    ),
                    format!("Add {user} to the group, then log out and back in so it applies:"),
                ];
                settings.commands = vec![usermod(user, group)];
            }
            None if !welcome.can_tune => settings.paragraphs = vec![format!("{reason}.")],
            _ => {}
        }
        if let Some(authorization) = &welcome.authorization {
            settings
                .facts
                .push(fact("Authorized", authorized(authorization)));
        }
        if let Session::Open { socket, .. } = &self.session {
            settings.facts.extend([
                fact("prismd version", welcome.prismd_version.clone()),
                fact("Protocol", welcome.protocol.to_string()),
                fact("Socket", socket.clone()),
            ]);
        }
    }

    /// The service action that fixes the connection natively, if any: Enable when prismd isn't
    /// running and systemd doesn't say it is, Restart when it failed. In Flatpak, Prism can't talk
    /// to systemd.
    pub(crate) fn service_action(&self) -> Option<ServiceAction> {
        let Some(PrismdHost {
            install: PrismdInstall::Native(unit),
            ..
        }) = &self.host
        else {
            return None;
        };
        match self.prismd() {
            PrismdConnection::NotConnected(NotConnected::NotRunning)
                if *unit != UnitState::Active =>
            {
                Some(ServiceAction::Enable)
            }
            PrismdConnection::NotConnected(NotConnected::Failed { .. }) => {
                Some(ServiceAction::Restart)
            }
            _ => None,
        }
    }

    /// The one command that sets prismd up from here, for the palette to copy, if any.
    pub(crate) fn setup_command(&self) -> Option<String> {
        Some(match self.prismd() {
            PrismdConnection::NotConnected(NotConnected::NotInstalled) => {
                let install = install_command(&self.host.as_ref()?.distro)?;
                format!("{install} && {ENABLE}")
            }
            PrismdConnection::NotConnected(NotConnected::NotRunning) => ENABLE.into(),
            PrismdConnection::NotConnected(NotConnected::Failed { .. }) => RESTART.into(),
            PrismdConnection::Connected(Welcome {
                authorization: Some(Authorization::NotInAdminGroup { user, group }),
                ..
            }) => usermod(&user, &group),
            _ => return None,
        })
    }
}

const AUR_INSTALL: &str = "yay -S prismd";

/// The command that adds `user` to `group`, which takes effect at their next login.
fn usermod(user: &str, group: &str) -> String {
    format!("sudo usermod -aG {group} {user}")
}

/// The command that installs prismd on `distro`, if Prism knows its package manager.
fn install_command(distro: &Distro) -> Option<&'static str> {
    let is = |names: &[&str]| {
        std::iter::once(&distro.id)
            .chain(&distro.id_like)
            .any(|id| names.contains(&id.as_str()))
    };
    Some(if is(&["debian", "ubuntu"]) {
        "sudo apt install prismd"
    } else if is(&["fedora", "rhel", "centos"]) {
        "sudo dnf install prismd"
    } else if is(&["suse", "opensuse"]) {
        "sudo zypper install prismd"
    } else if is(&["arch"]) {
        AUR_INSTALL
    } else {
        return None;
    })
}

/// What systemd's `Result` for a failed unit means, for the ones it commonly gives.
fn explain_result(result: &str) -> Option<&'static str> {
    Some(match result {
        "exit-code" => "it exited with an error",
        "signal" => "it was killed by a signal",
        "core-dump" => "it crashed",
        "watchdog" => "it stopped answering systemd's watchdog",
        "timeout" => "it took too long to start or stop",
        "start-limit-hit" => "it failed too often in a row",
        "resources" => "systemd couldn't set up what it needs",
        _ => return None,
    })
}

/// How prismd authorized the session's user, as Settings → Daemon says it.
fn authorized(authorization: &Authorization) -> String {
    match authorization {
        Authorization::Root {} => "as root".into(),
        Authorization::AdminUser { user } => {
            format!("as {user}, the admin_user in prismd's config")
        }
        Authorization::AdminGroup { user, group } => {
            format!("as {user}, a member of group {group}")
        }
        Authorization::NotInAdminGroup { user, group } => {
            format!("read-only: {user} isn't in group {group}")
        }
    }
}

fn fact(label: &str, value: impl Into<String>) -> Fact {
    Fact {
        label: label.into(),
        value: value.into(),
    }
}
