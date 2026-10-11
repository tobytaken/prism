//! View tests for desktop notifications: a prismd connection still lost after about 10 s notifies
//! through the fake notifier, and clicking the notification raises Prism at Settings → Daemon.

mod support;

use i_slint_backend_testing::AccessibleRole;
use prism_gpu::simulated::SimulatedFixture;
use prism_ui::{Notification, SettingsSection};
use slint::ComponentHandle;

use support::Harness;
use support::prismd::DaemonState;

use AccessibleRole::ListItem;

/// Prism with a session, its window as `focused` says, whose prismd has just stopped.
fn lost(focused: bool) -> Harness {
    let harness = Harness::with_daemon(SimulatedFixture::TunableGpu, DaemonState::Running);
    harness.activation.set_focused(focused);
    harness.tick();
    harness.prismd.stop_daemon();
    harness.app.receive_posted();
    harness
}

/// The one notification Prism sent.
fn the_notification(harness: &Harness) -> Notification {
    let [notification] = harness
        .notifier
        .sent()
        .try_into()
        .expect("one notification");
    notification
}

#[test]
fn a_lost_connection_still_lost_after_10_seconds_sends_one_notification() {
    let harness = lost(false);

    harness.ticks(9);
    assert_eq!(harness.notifier.sent(), []);
    harness.tick();

    let notification = the_notification(&harness);
    assert_eq!(notification.summary, "Prism lost its connection to prismd");
    assert_eq!(
        notification.body,
        "prismd isn't running. Tuning is read-only until Prism reconnects."
    );
    // Staying lost sends nothing more.
    harness.ticks(30);
    assert_eq!(harness.notifier.sent().len(), 1);
}

#[test]
fn an_outage_under_10_seconds_sends_nothing() {
    let harness = lost(false);

    harness.ticks(5);
    harness.prismd.start_daemon();
    harness.ticks(30);

    assert_eq!(harness.notifier.sent(), []);
}

#[test]
fn nothing_is_sent_while_prisms_window_has_focus() {
    let harness = lost(true);

    harness.ticks(15);

    assert_eq!(harness.notifier.sent(), []);
}

#[test]
fn reconnecting_withdraws_the_notification() {
    let harness = lost(false);
    harness.ticks(10);
    assert_eq!(harness.notifier.live().len(), 1);

    harness.prismd.start_daemon();
    harness.tick();

    assert_eq!(harness.notifier.live(), []);
}

#[test]
fn clicking_the_notification_raises_and_focuses_prism_at_settings_daemon() {
    let harness = lost(false);
    harness.ticks(10);
    let window = harness.app.window().window();
    window.set_minimized(true);

    harness
        .notifier
        .click(the_notification(&harness).id, Some("token-from-the-server"));
    harness.tick();

    assert!(window.is_visible());
    assert!(!window.is_minimized());
    assert_eq!(
        harness.activation.tokens(),
        [Some("token-from-the-server".to_owned())]
    );
    assert_eq!(
        harness.app.core().settings_section(),
        SettingsSection::Daemon
    );
    assert_eq!(
        harness.find(ListItem, "Daemon").accessible_item_selected(),
        Some(true)
    );
    assert!(harness.shows("prismd isn't running"));
}

#[test]
fn quitting_withdraws_the_notification() {
    let harness = lost(false);
    harness.ticks(10);

    harness.ctrl("q");

    assert_eq!(harness.notifier.live(), []);
}
