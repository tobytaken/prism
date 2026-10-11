//! Desktop notifications, starting with a lost prismd connection: none while Prism's window has
//! focus, none for an outage shorter than about 10 s, at most one live at a time about the same
//! thing, and clicking one raises Prism where it leads.

use std::time::Duration;

use prism_protocol::{DaemonSnapshot, ProtocolVersion, SessionMessage, Welcome};
use prism_ui::{
    AppearanceMode, Content, Core, Distro, Effect, Input, Intent, Notification, PrismdHost,
    PrismdInstall, Section, SessionFailure, SettingsSection, Timer, UnitState,
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

fn welcome() -> Welcome {
    Welcome {
        protocol: ProtocolVersion::CURRENT,
        prismd_version: "0.1.0".into(),
        can_tune: true,
        reason: None,
        authorization: None,
    }
}

const LOSS: Effect = Effect::StartTimer {
    timer: Timer::ConnectionLost,
    after: Duration::from_secs(10),
};

/// Prism connected to prismd, with its window as `focused` says.
fn connected(focused: bool) -> Core {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    connect(&mut core);
    core.handle(Input::WindowFocusChanged(focused));
    core
}

fn connect(core: &mut Core) -> Vec<Effect> {
    core.handle(Input::PrismdConnected {
        welcome: welcome(),
        snapshot: snapshot(),
        socket: "/run/prism/prismd.sock".into(),
    })
}

fn lose(core: &mut Core) -> Vec<Effect> {
    core.handle(Input::PrismdNotConnected(SessionFailure::Unavailable {
        message: "Connection reset by peer (os error 104)".into(),
    }))
}

/// systemd says prismd failed.
fn failed(core: &mut Core) {
    core.handle(Input::PrismdHostChecked(PrismdHost {
        install: PrismdInstall::Native(UnitState::Failed {
            result: "exit-code".into(),
        }),
        distro: Distro::default(),
    }));
}

fn elapse(core: &mut Core) -> Vec<Effect> {
    core.handle(Input::TimerElapsed(Timer::ConnectionLost))
}

/// The notifications among `effects`.
fn sent(effects: &[Effect]) -> Vec<Notification> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Notify(notification) => Some(notification.clone()),
            _ => None,
        })
        .collect()
}

/// The connection lost, still lost when the timer elapses: the one notification about it.
fn still_lost(core: &mut Core) -> Notification {
    lose(core);
    failed(core);
    let effects = elapse(core);
    let [notification] = sent(&effects).try_into().expect("one notification");
    notification
}

#[test]
fn a_lost_connection_starts_the_10_second_wait_without_notifying_yet() {
    let mut core = connected(false);

    let effects = lose(&mut core);

    assert!(effects.contains(&LOSS), "{effects:?}");
    assert_eq!(sent(&effects), []);
}

#[test]
fn a_connection_still_lost_after_10_seconds_notifies_once_with_its_reason() {
    let mut core = connected(false);

    let notification = still_lost(&mut core);

    assert_eq!(notification.summary, "Prism lost its connection to prismd");
    assert_eq!(
        notification.body,
        "prismd failed. Tuning is read-only until Prism reconnects."
    );
    assert_eq!(notification.replaces, None);
    // Failing to reconnect again and again doesn't restart the wait or notify again.
    assert_eq!(lose(&mut core), []);
    assert_eq!(sent(&elapse(&mut core)), []);
}

#[test]
fn an_outage_shorter_than_10_seconds_doesnt_notify() {
    let mut core = connected(false);
    lose(&mut core);

    let effects = connect(&mut core);

    assert!(effects.contains(&Effect::StopTimer(Timer::ConnectionLost)));
    // Even if the timer had already fired on its way in.
    assert_eq!(elapse(&mut core), []);
}

#[test]
fn prism_never_notifies_while_its_window_has_focus() {
    let mut core = connected(true);
    lose(&mut core);

    assert_eq!(elapse(&mut core), []);
}

#[test]
fn focus_counts_when_the_wait_ends_not_when_the_connection_is_lost() {
    let mut core = connected(true);
    lose(&mut core);
    core.handle(Input::WindowFocusChanged(false));

    assert_eq!(sent(&elapse(&mut core)).len(), 1);
}

#[test]
fn a_window_that_never_reported_focus_counts_as_unfocused() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    connect(&mut core);
    lose(&mut core);

    assert_eq!(sent(&elapse(&mut core)).len(), 1);
}

#[test]
fn no_prismd_from_the_start_isnt_a_lost_connection() {
    let mut core = Core::default();
    core.handle(Input::Launched {
        appearance: AppearanceMode::System,
    });
    core.handle(Input::WindowFocusChanged(false));

    let effects = lose(&mut core);

    assert!(!effects.contains(&LOSS), "{effects:?}");
    assert_eq!(elapse(&mut core), []);
}

#[test]
fn reconnecting_withdraws_the_notification() {
    let mut core = connected(false);
    let notification = still_lost(&mut core);

    let effects = connect(&mut core);

    assert!(
        effects.contains(&Effect::WithdrawNotification(notification.id)),
        "{effects:?}"
    );
}

#[test]
fn clicking_the_notification_raises_prism_at_settings_daemon_with_the_servers_token() {
    let mut core = connected(false);
    let notification = still_lost(&mut core);

    let effects = core.handle(Input::NotificationClicked {
        id: notification.id,
        activation_token: Some("token-from-the-server".into()),
    });

    assert!(effects.contains(&Effect::RaiseWindow {
        activation_token: Some("token-from-the-server".into())
    }));
    assert_eq!(core.content(), Content::Section(Section::Settings));
    assert_eq!(core.settings_section(), SettingsSection::Daemon);
    // The server closes a clicked notification, so reconnecting has nothing to withdraw.
    assert!(
        !connect(&mut core)
            .iter()
            .any(|effect| matches!(effect, Effect::WithdrawNotification(_)))
    );
}

#[test]
fn quitting_withdraws_the_live_notifications() {
    let mut core = connected(false);
    let notification = still_lost(&mut core);

    let effects = core.handle(Input::Intent(Intent::CloseWindow));

    assert_eq!(
        effects,
        [Effect::WithdrawNotification(notification.id), Effect::Quit]
    );
}

#[test]
fn a_notification_closed_by_the_server_is_no_longer_live() {
    let mut core = connected(false);
    let notification = still_lost(&mut core);

    core.handle(Input::NotificationClosed(notification.id));

    assert_eq!(
        core.handle(Input::Intent(Intent::CloseWindow)),
        [Effect::Quit]
    );
}
