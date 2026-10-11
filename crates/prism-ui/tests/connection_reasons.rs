//! Why there is no prismd connection: derived from what connecting said, what the host says about
//! prismd (systemd's unit state natively, whether `/run/prism` is visible in Flatpak) and the
//! handshake.

use prism_protocol::{DaemonSnapshot, Incompatible, ProtocolVersion, SessionMessage, Welcome};
use prism_ui::{
    AppearanceMode, Core, Distro, Effect, Input, NotConnected, PrismdConnection, PrismdHost,
    PrismdInstall, SessionFailure, TuningAccess, UnitState,
};

/// The snapshot from the shared contract, which describes the simulated tunable GPU.
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

fn welcome(can_tune: bool) -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune,
        reason: (!can_tune).then(|| "bob isn't in group wheel".into()),
        authorization: None,
    }
}

fn incompatible() -> Incompatible {
    Incompatible {
        protocol: ProtocolVersion { major: 0, minor: 9 },
        prismd_version: "0.9.0".into(),
    }
}

fn ubuntu() -> Distro {
    Distro {
        id: "ubuntu".into(),
        id_like: vec!["debian".into()],
        name: "Ubuntu 24.04 LTS".into(),
    }
}

fn native(unit: UnitState) -> PrismdHost {
    PrismdHost {
        install: PrismdInstall::Native(unit),
        distro: ubuntu(),
    }
}

fn flatpak(run_dir_visible: bool) -> PrismdHost {
    PrismdHost {
        install: PrismdInstall::Flatpak { run_dir_visible },
        distro: ubuntu(),
    }
}

fn launched() -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core
}

fn unavailable() -> Input {
    Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "No such file or directory (os error 2)".into(),
    })
}

/// What connecting to prismd said.
#[derive(Debug, Clone, Copy)]
enum Handshake {
    /// No socket, or nothing answered on it.
    Unavailable,
    Incompatible,
    Welcomed,
    WelcomedReadOnly,
}

fn input(handshake: Handshake) -> Input {
    match handshake {
        Handshake::Unavailable => unavailable(),
        Handshake::Incompatible => {
            Input::PrismdNotConnected(SessionFailure::Incompatible(incompatible()))
        }
        Handshake::Welcomed | Handshake::WelcomedReadOnly => Input::PrismdConnected {
            welcome: welcome(matches!(handshake, Handshake::Welcomed)),
            snapshot: snapshot(),
            socket: "/run/prism/prismd.sock".into(),
        },
    }
}

fn not_connected(reason: NotConnected) -> PrismdConnection {
    PrismdConnection::NotConnected(reason)
}

#[test]
fn every_combination_of_handshake_and_host_decides_the_reason() {
    let failed = || UnitState::Failed {
        result: "exit-code".into(),
    };
    let hosts = [
        ("not checked yet", None),
        ("unit not found", Some(native(UnitState::NotFound))),
        (
            "unit inactive",
            Some(native(UnitState::Inactive { enabled: false })),
        ),
        ("unit active", Some(native(UnitState::Active))),
        ("unit failed", Some(native(failed()))),
        (
            "systemctl unusable",
            Some(native(UnitState::Unknown {
                message: "systemctl: not found".into(),
            })),
        ),
        ("Flatpak without /run/prism", Some(flatpak(false))),
        ("Flatpak with /run/prism", Some(flatpak(true))),
    ];
    for (host_name, host) in hosts {
        let unavailable_reason = match host_name {
            "unit not found" | "Flatpak without /run/prism" => NotConnected::NotInstalled,
            "unit failed" => NotConnected::Failed {
                result: "exit-code".into(),
            },
            // Active but not answering, or no way to tell: prismd isn't there to talk to yet.
            _ => NotConnected::NotRunning,
        };
        for (handshake, expected) in [
            (Handshake::Unavailable, not_connected(unavailable_reason)),
            (
                Handshake::Incompatible,
                not_connected(NotConnected::Incompatible(incompatible())),
            ),
            (
                Handshake::Welcomed,
                PrismdConnection::Connected(welcome(true)),
            ),
            (
                Handshake::WelcomedReadOnly,
                PrismdConnection::Connected(welcome(false)),
            ),
        ] {
            let mut core = launched();
            core.handle(input(handshake));
            if let Some(host) = host.clone() {
                core.handle(Input::PrismdHostChecked(host));
            }

            assert_eq!(core.prismd(), expected, "{handshake:?} with {host_name}");
        }
    }
}

#[test]
fn losing_the_session_checks_the_host_but_failed_reconnects_dont() {
    let mut core = launched();
    assert_eq!(
        core.handle(unavailable()),
        [Effect::StartSampler, Effect::CheckPrismdHost]
    );
    core.handle(Input::PrismdHostChecked(native(UnitState::Inactive {
        enabled: false,
    })));
    // The connector reports a reason once until it changes; the same one again checks nothing.
    assert_eq!(core.handle(unavailable()), []);

    core.handle(input(Handshake::Welcomed));
    assert_eq!(
        core.handle(unavailable()),
        [Effect::StartSampler, Effect::CheckPrismdHost]
    );
}

#[test]
fn an_incompatible_prismd_needs_no_host_check() {
    let mut core = launched();

    let effects = core.handle(input(Handshake::Incompatible));

    assert_eq!(effects, [Effect::StartSampler]);
}

#[test]
fn a_connect_overrides_the_unit_state_and_a_later_loss_doesnt_trust_the_old_one() {
    let mut core = launched();
    core.handle(unavailable());
    core.handle(Input::PrismdHostChecked(native(UnitState::NotFound)));
    assert_eq!(core.prismd(), not_connected(NotConnected::NotInstalled));

    core.handle(input(Handshake::Welcomed));
    assert!(matches!(core.prismd(), PrismdConnection::Connected(_)));

    // Until the new check answers, the old "not found" no longer holds.
    core.handle(unavailable());
    assert_eq!(core.prismd(), not_connected(NotConnected::NotRunning));
    core.handle(Input::PrismdHostChecked(native(UnitState::Failed {
        result: "watchdog".into(),
    })));
    assert_eq!(
        core.prismd(),
        not_connected(NotConnected::Failed {
            result: "watchdog".into()
        })
    );
}

#[test]
fn a_host_check_answering_after_a_connect_is_ignored() {
    let mut core = launched();
    core.handle(unavailable());
    core.handle(input(Handshake::Welcomed));

    core.handle(Input::PrismdHostChecked(native(UnitState::NotFound)));
    core.handle(unavailable());

    assert_eq!(core.prismd(), not_connected(NotConnected::NotRunning));
}

#[test]
fn tuning_access_gives_each_reason_in_short() {
    let read_only = |reason: &str| TuningAccess::ReadOnly {
        reason: reason.into(),
    };
    for (host, reason) in [
        (native(UnitState::NotFound), "not installed"),
        (
            native(UnitState::Inactive { enabled: false }),
            "not running",
        ),
        (
            native(UnitState::Failed {
                result: "exit-code".into(),
            }),
            "failed",
        ),
        (flatpak(false), "not installed"),
        (flatpak(true), "not running"),
    ] {
        let mut core = launched();
        core.handle(unavailable());
        core.handle(Input::PrismdHostChecked(host));
        assert_eq!(core.tuning_access(), read_only(reason));
    }

    let mut core = launched();
    assert_eq!(core.tuning_access(), read_only("connecting"));
    core.handle(input(Handshake::Incompatible));
    assert_eq!(core.tuning_access(), read_only("incompatible"));
}

#[test]
fn the_status_bar_item_shows_the_reason() {
    let mut core = launched();
    assert_eq!(core.status_bar().connection, "PRISMD · CONNECTING");

    core.handle(unavailable());
    core.handle(Input::PrismdHostChecked(native(UnitState::NotFound)));
    assert_eq!(core.status_bar().connection, "PRISMD · NOT INSTALLED");
    assert!(!core.status_bar().connected);

    core.handle(input(Handshake::WelcomedReadOnly));
    assert_eq!(core.status_bar().connection, "PRISMD · NOT ALLOWED");
    assert!(core.status_bar().connected);

    core.handle(input(Handshake::Welcomed));
    assert_eq!(core.status_bar().connection, "PRISMD · CONNECTED");
}
