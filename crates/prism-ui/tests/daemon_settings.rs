//! Settings → Daemon: why there's no tuning access and how to fix it, for each reason, and what
//! the connection is when there is one.

use prism_protocol::{
    Authorization, DaemonSnapshot, Incompatible, ProtocolVersion, SessionMessage, Welcome,
};
use prism_ui::{
    AppearanceMode, Core, DaemonAction, DaemonSettings, Distro, Fact, Input, PrismdHost,
    PrismdInstall, ServiceAction, SessionFailure, UnitState,
};

const ENABLE: &str = "sudo systemctl enable --now prismd";
const WHY_PRISMD: &str = "Tuning, fan curves and game switching need prismd, Prism's host daemon. Monitoring works \
     without it.";

fn snapshot() -> DaemonSnapshot {
    let message: SessionMessage = serde_json::from_str(include_str!(
        "../../prism-protocol/tests/contracts/daemon-snapshot.json"
    ))
    .unwrap();
    let SessionMessage::Snapshot { snapshot } = message else {
        panic!("the contract holds a snapshot")
    };
    snapshot
}

fn distro(id: &str, id_like: &[&str], name: &str) -> Distro {
    Distro {
        id: id.into(),
        id_like: id_like.iter().map(|like| (*like).into()).collect(),
        name: name.into(),
    }
}

fn ubuntu() -> Distro {
    distro("ubuntu", &["debian"], "Ubuntu 24.04 LTS")
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core
}

/// A core without a session, whose host says `install` on `distro`.
fn unavailable(install: PrismdInstall, distro: Distro) -> Core {
    let mut core = launched();
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));
    core.handle(Input::PrismdHostChecked(PrismdHost { install, distro }));
    core
}

fn native(unit: UnitState) -> Core {
    unavailable(PrismdInstall::Native(unit), ubuntu())
}

fn flatpak(run_dir_visible: bool) -> Core {
    unavailable(PrismdInstall::Flatpak { run_dir_visible }, ubuntu())
}

fn connected(can_tune: bool, authorization: Option<Authorization>) -> Core {
    let mut core = launched();
    core.handle(Input::PrismdConnected {
        welcome: Welcome {
            protocol: ProtocolVersion { major: 0, minor: 1 },
            prismd_version: "0.1.4".into(),
            can_tune,
            reason: (!can_tune).then(|| "bob isn't in group wheel".into()),
            authorization,
        },
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });
    core
}

fn fact(label: &str, value: &str) -> Fact {
    Fact {
        label: label.into(),
        value: value.into(),
    }
}

fn strings(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).into()).collect()
}

/// Daemon settings with only a title, to fill in with `..`.
fn titled(title: &str) -> DaemonSettings {
    DaemonSettings {
        title: title.into(),
        paragraphs: Vec::new(),
        facts: Vec::new(),
        commands: Vec::new(),
        action: None,
        error: None,
    }
}

#[test]
fn not_installed_gives_the_install_commands_for_the_host_s_distribution() {
    for (distro, install) in [
        (ubuntu(), "sudo apt install prismd"),
        (
            distro("debian", &[], "Debian GNU/Linux 12 (bookworm)"),
            "sudo apt install prismd",
        ),
        (
            distro("fedora", &[], "Fedora Linux 42"),
            "sudo dnf install prismd",
        ),
        (
            distro("almalinux", &["rhel", "centos", "fedora"], "AlmaLinux 9.4"),
            "sudo dnf install prismd",
        ),
        (
            distro(
                "opensuse-tumbleweed",
                &["opensuse", "suse"],
                "openSUSE Tumbleweed",
            ),
            "sudo zypper install prismd",
        ),
    ] {
        let name = distro.name.clone();
        let core = unavailable(PrismdInstall::Native(UnitState::NotFound), distro);

        let settings = core.daemon_settings();

        assert_eq!(settings.title, "prismd isn't installed", "{name}");
        assert_eq!(
            settings.paragraphs,
            [
                WHY_PRISMD.to_owned(),
                format!("Install it on {name}, then start it:")
            ],
            "{name}"
        );
        assert_eq!(settings.commands, strings(&[install, ENABLE]), "{name}");
        assert_eq!(settings.action, None, "{name}");
    }
}

#[test]
fn not_installed_on_arch_installs_from_the_aur() {
    for distro in [
        distro("arch", &[], "Arch Linux"),
        distro("cachyos", &["arch"], "CachyOS"),
    ] {
        let core = unavailable(PrismdInstall::Native(UnitState::NotFound), distro);

        let settings = core.daemon_settings();

        assert_eq!(
            settings.paragraphs,
            strings(&[
                WHY_PRISMD,
                "Install it from the AUR, such as with yay, then start it:"
            ])
        );
        assert_eq!(settings.commands, strings(&["yay -S prismd", ENABLE]));
    }
}

#[test]
fn not_installed_on_a_distribution_prism_doesnt_know_says_to_install_the_package() {
    let core = unavailable(
        PrismdInstall::Native(UnitState::NotFound),
        distro("gentoo", &[], "Gentoo Linux"),
    );

    assert_eq!(
        core.daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&[
                WHY_PRISMD,
                "Install the prismd package for Gentoo Linux, then start it:"
            ]),
            commands: strings(&[ENABLE]),
            ..titled("prismd isn't installed")
        }
    );
}

#[test]
fn not_installed_in_flatpak_says_to_restart_prism_after_installing() {
    let settings = flatpak(false).daemon_settings();

    assert_eq!(
        settings,
        DaemonSettings {
            paragraphs: strings(&[
                WHY_PRISMD,
                "Install it on Ubuntu 24.04 LTS, then start it:",
                "Just installed it? Restart Prism.",
            ]),
            commands: strings(&["sudo apt install prismd", ENABLE]),
            ..titled("prismd isn't installed")
        }
    );
}

#[test]
fn not_running_natively_offers_to_enable_prismd() {
    assert_eq!(
        native(UnitState::Inactive { enabled: true })
            .daemon_settings()
            .paragraphs,
        strings(&["prismd is enabled, but has stopped."])
    );
    assert_eq!(
        native(UnitState::Inactive { enabled: false }).daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&["prismd is installed, but isn't enabled."]),
            action: Some(DaemonAction {
                action: ServiceAction::Enable,
                title: "Enable prismd".into(),
                busy: false,
            }),
            ..titled("prismd isn't running")
        }
    );
}

#[test]
fn a_prismd_systemd_says_is_running_but_doesnt_answer_yet_needs_no_enabling() {
    // As right after Enable, until Prism's next attempt to connect.
    assert_eq!(
        native(UnitState::Active).daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&[
                "systemd says prismd is running, but it isn't answering yet. Prism connects as \
                 soon as it does."
            ]),
            ..titled("prismd isn't running")
        }
    );
}

#[test]
fn not_running_in_flatpak_gives_the_command_to_copy() {
    assert_eq!(
        flatpak(true).daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&[
                "prismd is installed, but isn't enabled or has stopped. Start it on the host:"
            ]),
            commands: strings(&[ENABLE]),
            ..titled("prismd isn't running")
        }
    );
}

#[test]
fn failed_gives_systemd_s_result_offers_a_restart_and_the_journal_command() {
    assert_eq!(
        native(UnitState::Failed {
            result: "watchdog".into()
        })
        .daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&[
                "systemd gave up on it with result watchdog: it stopped answering systemd's \
                 watchdog.",
                "Its log says why:",
            ]),
            commands: strings(&["journalctl -u prismd -b"]),
            action: Some(DaemonAction {
                action: ServiceAction::Restart,
                title: "Restart prismd".into(),
                busy: false,
            }),
            ..titled("prismd failed")
        }
    );
    let explained = |result: &str| {
        native(UnitState::Failed {
            result: result.into(),
        })
        .daemon_settings()
        .paragraphs[0]
            .clone()
    };
    assert_eq!(
        explained("exit-code"),
        "systemd gave up on it with result exit-code: it exited with an error."
    );
    assert_eq!(
        explained("brand-new"),
        "systemd gave up on it with result brand-new."
    );
}

#[test]
fn incompatible_gives_both_versions_and_which_side_to_update() {
    let prism = format!(
        "{} · protocol {}",
        env!("CARGO_PKG_VERSION"),
        ProtocolVersion::CURRENT
    );
    for (protocol, prismd, update) in [
        (
            ProtocolVersion { major: 9, minor: 0 },
            "9.0.0 · protocol 9.0",
            "prismd is newer than Prism: update Prism.",
        ),
        (
            ProtocolVersion { major: 0, minor: 0 },
            "0.0.1 · protocol 0.0",
            "prismd is older than Prism: update prismd.",
        ),
    ] {
        let mut core = launched();
        core.handle(Input::PrismdNotConnected(SessionFailure::Incompatible(
            Incompatible {
                protocol,
                prismd_version: prismd.split(' ').next().unwrap().into(),
            },
        )));

        assert_eq!(
            core.daemon_settings(),
            DaemonSettings {
                paragraphs: strings(&[update]),
                facts: vec![
                    fact("Prism version", &prism),
                    fact("prismd version", prismd)
                ],
                ..titled("prismd speaks another protocol version")
            }
        );
    }
}

#[test]
fn connected_shows_tuning_access_the_versions_how_the_user_is_authorized_and_the_socket() {
    for (authorization, authorized) in [
        (Authorization::Root {}, "as root"),
        (
            Authorization::AdminUser {
                user: "alice".into(),
            },
            "as alice, the admin_user in prismd's config",
        ),
        (
            Authorization::AdminGroup {
                user: "alice".into(),
                group: "wheel".into(),
            },
            "as alice, a member of group wheel",
        ),
    ] {
        assert_eq!(
            connected(true, Some(authorization)).daemon_settings(),
            DaemonSettings {
                facts: vec![
                    fact("Tuning access", "Yes"),
                    fact("Authorized", authorized),
                    fact("prismd version", "0.1.4"),
                    fact("Protocol", "0.1"),
                    fact("Socket", "/run/prism/prismd.sock"),
                ],
                ..titled("Connected to prismd")
            }
        );
    }
}

#[test]
fn not_allowed_names_the_group_and_gives_the_usermod_command() {
    let core = connected(
        false,
        Some(Authorization::NotInAdminGroup {
            user: "bob".into(),
            group: "wheel".into(),
        }),
    );

    assert_eq!(
        core.daemon_settings(),
        DaemonSettings {
            paragraphs: strings(&[
                "User bob isn't in group wheel, so prismd lets this Prism monitor GPUs but not \
                 tune them.",
                "Add bob to the group, then log out and back in so it applies:",
            ]),
            commands: strings(&["sudo usermod -aG wheel bob"]),
            facts: vec![
                fact("Tuning access", "No: bob isn't in group wheel"),
                fact("Authorized", "read-only: bob isn't in group wheel"),
                fact("prismd version", "0.1.4"),
                fact("Protocol", "0.1"),
                fact("Socket", "/run/prism/prismd.sock"),
            ],
            ..titled("No tuning access")
        }
    );
}

#[test]
fn not_allowed_without_prismd_saying_how_gives_its_reason_alone() {
    let settings = connected(false, None).daemon_settings();

    assert_eq!(settings.title, "No tuning access");
    assert_eq!(settings.paragraphs, strings(&["bob isn't in group wheel."]));
    assert!(settings.commands.is_empty());
    assert!(!settings.facts.iter().any(|fact| fact.label == "Authorized"));
}

#[test]
fn connecting_says_so() {
    assert_eq!(
        launched().daemon_settings(),
        titled("Connecting to prismd…")
    );
}
