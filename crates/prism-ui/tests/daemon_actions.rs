//! Fixing the prismd connection from Prism: Enable and Restart through systemd under polkit,
//! copying commands, and the palette's prismd commands.

use prism_protocol::{Authorization, DaemonSnapshot, ProtocolVersion, SessionMessage, Welcome};
use prism_ui::{
    AppearanceMode, Core, Distro, Effect, Input, Intent, PrismdHost, PrismdInstall, ServiceAction,
    ServiceOutcome, SessionFailure, UnitState,
};

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

fn ubuntu() -> Distro {
    Distro {
        id: "ubuntu".into(),
        id_like: vec!["debian".into()],
        name: "Ubuntu 24.04 LTS".into(),
    }
}

fn unavailable(install: PrismdInstall) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    }));
    core.handle(Input::PrismdHostChecked(PrismdHost {
        install,
        distro: ubuntu(),
    }));
    core
}

fn native(unit: UnitState) -> Core {
    unavailable(PrismdInstall::Native(unit))
}

fn not_running() -> Core {
    native(UnitState::Inactive)
}

fn intent(core: &mut Core, intent: Intent) -> Vec<Effect> {
    core.handle(Input::Intent(intent))
}

fn done(core: &mut Core, action: ServiceAction, outcome: ServiceOutcome) -> Vec<Effect> {
    core.handle(Input::PrismdServiceDone { action, outcome })
}

/// The palette's prismd commands, as (title, why it can't run).
fn prismd_commands(core: &mut Core) -> Vec<(String, Option<String>)> {
    intent(core, Intent::OpenPalette);
    let entries = core
        .palette()
        .unwrap()
        .entries
        .into_iter()
        .filter(|entry| entry.title.contains("prismd"))
        .map(|entry| (entry.title, entry.unavailable))
        .collect();
    intent(core, Intent::Dismiss);
    entries
}

/// Runs the palette command titled `title`.
fn run_command(core: &mut Core, title: &str) -> Vec<Effect> {
    intent(core, Intent::OpenPalette);
    let index = core
        .palette()
        .unwrap()
        .entries
        .iter()
        .position(|entry| entry.title == title)
        .unwrap_or_else(|| panic!("no command is titled {title:?}"));
    intent(core, Intent::RunPaletteEntry(index))
}

#[test]
fn enable_runs_once_until_it_ends_then_checks_the_host_again() {
    let mut core = not_running();

    assert_eq!(
        intent(&mut core, Intent::RunService(ServiceAction::Enable)),
        [Effect::RunService(ServiceAction::Enable)]
    );
    let action = core.daemon_settings().action.unwrap();
    assert_eq!(action.title, "Enabling prismd…");
    assert!(action.busy);
    assert_eq!(
        intent(&mut core, Intent::RunService(ServiceAction::Enable)),
        []
    );

    assert_eq!(
        done(&mut core, ServiceAction::Enable, ServiceOutcome::Done),
        [Effect::CheckPrismdHost]
    );
    assert!(!core.daemon_settings().action.unwrap().busy);
    assert_eq!(core.daemon_settings().error, None);
}

#[test]
fn canceling_polkit_s_prompt_leaves_everything_as_it_was() {
    let mut core = not_running();
    let before = core.daemon_settings();
    intent(&mut core, Intent::RunService(ServiceAction::Enable));

    let effects = done(&mut core, ServiceAction::Enable, ServiceOutcome::Canceled);

    assert_eq!(effects, []);
    assert_eq!(core.daemon_settings(), before);
}

#[test]
fn a_failed_service_action_says_why_until_the_next_attempt() {
    let mut core = native(UnitState::Failed {
        result: "exit-code".into(),
    });
    intent(&mut core, Intent::RunService(ServiceAction::Restart));

    let effects = done(
        &mut core,
        ServiceAction::Restart,
        ServiceOutcome::Failed {
            message: "Job for prismd.service failed.".into(),
        },
    );

    assert_eq!(effects, [Effect::CheckPrismdHost]);
    assert_eq!(
        core.daemon_settings().error.as_deref(),
        Some("Couldn't restart prismd: Job for prismd.service failed.")
    );
    intent(&mut core, Intent::RunService(ServiceAction::Restart));
    assert_eq!(core.daemon_settings().error, None);
}

#[test]
fn a_service_action_that_doesnt_apply_does_nothing() {
    // Restart is for a failed prismd, and Flatpak can't talk to systemd at all.
    assert_eq!(
        intent(
            &mut not_running(),
            Intent::RunService(ServiceAction::Restart)
        ),
        []
    );
    let mut flatpak = unavailable(PrismdInstall::Flatpak {
        run_dir_visible: true,
    });
    assert_eq!(
        intent(&mut flatpak, Intent::RunService(ServiceAction::Enable)),
        []
    );
}

#[test]
fn copying_a_command_puts_it_on_the_clipboard() {
    let mut core = not_running();

    let effects = intent(&mut core, Intent::Copy("journalctl -u prismd -b".into()));

    assert_eq!(effects, [Effect::Copy("journalctl -u prismd -b".into())]);
}

#[test]
fn the_palette_offers_enable_or_restart_natively_when_they_apply() {
    assert_eq!(
        prismd_commands(&mut not_running()),
        [
            ("Enable prismd".to_owned(), None),
            ("Copy prismd setup command".to_owned(), None),
        ]
    );
    let mut failed = native(UnitState::Failed {
        result: "watchdog".into(),
    });
    assert_eq!(
        prismd_commands(&mut failed),
        [
            ("Restart prismd".to_owned(), None),
            ("Copy prismd setup command".to_owned(), None),
        ]
    );
    let mut flatpak = unavailable(PrismdInstall::Flatpak {
        run_dir_visible: true,
    });
    assert_eq!(
        prismd_commands(&mut flatpak),
        [("Copy prismd setup command".to_owned(), None)]
    );

    let mut core = not_running();
    assert_eq!(
        run_command(&mut core, "Enable prismd"),
        [Effect::RunService(ServiceAction::Enable)]
    );
    assert_eq!(
        prismd_commands(&mut core)[0],
        (
            "Enable prismd".to_owned(),
            Some("Enabling prismd…".to_owned())
        )
    );
}

#[test]
fn copy_prismd_setup_command_copies_what_fixes_the_reason() {
    for (mut core, command) in [
        (
            native(UnitState::NotFound),
            "sudo apt install prismd && sudo systemctl enable --now prismd",
        ),
        (not_running(), "sudo systemctl enable --now prismd"),
        (
            native(UnitState::Failed {
                result: "exit-code".into(),
            }),
            "sudo systemctl restart prismd",
        ),
    ] {
        assert_eq!(
            run_command(&mut core, "Copy prismd setup command"),
            [Effect::Copy(command.into())]
        );
    }

    let mut not_allowed = Core::default();
    not_allowed.handle(Input::PrismdConnected {
        welcome: Welcome {
            protocol: ProtocolVersion::CURRENT,
            prismd_version: "0.1.0".into(),
            can_tune: false,
            reason: Some("bob isn't in group wheel".into()),
            authorization: Some(Authorization::NotInAdminGroup {
                user: "bob".into(),
                group: "wheel".into(),
            }),
        },
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });
    assert_eq!(
        run_command(&mut not_allowed, "Copy prismd setup command"),
        [Effect::Copy("sudo usermod -aG wheel bob".into())]
    );
}

#[test]
fn with_tuning_access_there_are_no_prismd_commands() {
    let mut core = Core::default();
    core.handle(Input::PrismdConnected {
        welcome: Welcome {
            protocol: ProtocolVersion::CURRENT,
            prismd_version: "0.1.0".into(),
            can_tune: true,
            reason: None,
            authorization: Some(Authorization::Root {}),
        },
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    });

    assert_eq!(prismd_commands(&mut core), []);
}
