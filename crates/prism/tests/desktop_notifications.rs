//! Desktop notifications on a private session bus, with a fake `org.freedesktop.Notifications`
//! server: what Prism sends, replacing and withdrawing, and the server's activation token reaching
//! Prism with a click.

mod support;

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use prism::{DesktopNotifier, NotificationEvent, Notifier};
use prism_ui::{Notification, NotificationId};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use support::bus::PrivateBus;

const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const WAIT: Duration = Duration::from_secs(5);

/// A `Notify` call the server took, with the id it answered.
#[derive(Debug)]
struct Shown {
    id: u32,
    app_name: String,
    replaces_id: u32,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    desktop_entry: Option<String>,
}

#[derive(Debug)]
enum Call {
    Notify(Shown),
    Close(u32),
}

/// The notification server, which numbers notifications from 1 and records what it's asked.
struct FakeServer {
    calls: Sender<Call>,
    last: u32,
    /// Whether the user clicks each notification before the server has even answered `Notify`.
    click_at_once: bool,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeServer {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &mut self,
        #[zbus(connection)] connection: &zbus::Connection,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        // As the spec has it, a replacement keeps the id of the notification it replaces.
        let id = if replaces_id == 0 {
            self.last += 1;
            self.last
        } else {
            replaces_id
        };
        let desktop_entry = hints
            .get("desktop-entry")
            .and_then(|entry| String::try_from(entry.clone()).ok());
        let _ = self.calls.send(Call::Notify(Shown {
            id,
            app_name,
            replaces_id,
            app_icon,
            summary,
            body,
            actions,
            desktop_entry,
        }));
        if self.click_at_once {
            // A click, as GNOME sends it: the token, the action, then the closing.
            let _ = connection
                .emit_signal(
                    None::<()>,
                    PATH,
                    INTERFACE,
                    "ActivationToken",
                    &(id, "early"),
                )
                .await;
            let _ = connection
                .emit_signal(
                    None::<()>,
                    PATH,
                    INTERFACE,
                    "ActionInvoked",
                    &(id, "default"),
                )
                .await;
            let _ = connection
                .emit_signal(
                    None::<()>,
                    PATH,
                    INTERFACE,
                    "NotificationClosed",
                    &(id, 2u32),
                )
                .await;
        }
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.calls.send(Call::Close(id));
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }
}

struct Desktop {
    _bus: PrivateBus,
    server: Connection,
    calls: Receiver<Call>,
    /// Until Prism quits.
    notifier: Option<DesktopNotifier>,
    /// The notifier's unique name, for signals the server sends only to it.
    notifier_name: String,
    events: Receiver<NotificationEvent>,
}

impl Desktop {
    fn start() -> Self {
        Self::with_server(false)
    }

    /// With a server on which the user clicks each notification before it has even answered.
    fn clicking_at_once() -> Self {
        Self::with_server(true)
    }

    fn with_server(click_at_once: bool) -> Self {
        let bus = PrivateBus::start();
        let server = bus.connect();
        let (sender, calls) = mpsc::channel();
        server
            .object_server()
            .at(
                PATH,
                FakeServer {
                    calls: sender,
                    last: 0,
                    click_at_once,
                },
            )
            .unwrap();
        server.request_name(INTERFACE).unwrap();
        let connection = bus.connect();
        let notifier_name = connection.unique_name().unwrap().to_string();
        let notifier = Some(DesktopNotifier::with_connection(connection));
        let (sender, events) = mpsc::channel();
        notifier.as_ref().unwrap().listen(Box::new(move |event| {
            let _ = sender.send(event);
        }));
        Self {
            _bus: bus,
            server,
            calls,
            notifier,
            notifier_name,
            events,
        }
    }

    fn call(&self) -> Call {
        self.calls
            .recv_timeout(WAIT)
            .expect("the server was called")
    }

    fn notifier(&self) -> &DesktopNotifier {
        self.notifier.as_ref().expect("Prism is running")
    }

    /// Sends `notification` and waits for the server to show it.
    fn show(&self, notification: Notification) -> Shown {
        self.notifier().notify(notification);
        match self.call() {
            Call::Notify(shown) => shown,
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    /// Emits the server's `signal` with `body`, to everyone, or only to the notifier.
    fn signal(
        &self,
        targeted: bool,
        signal: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) {
        let destination = targeted.then_some(self.notifier_name.as_str());
        self.server
            .emit_signal(destination, PATH, INTERFACE, signal, body)
            .unwrap();
    }

    fn event(&self) -> NotificationEvent {
        self.events.recv_timeout(WAIT).expect("Prism heard of it")
    }

    fn no_event(&self) {
        assert!(
            self.events
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "Prism heard of something"
        );
    }
}

fn notification(id: u64, replaces: Option<u64>) -> Notification {
    Notification {
        id: NotificationId(id),
        replaces: replaces.map(NotificationId),
        summary: "Prism lost its connection to prismd".into(),
        body: "prismd isn't running. Tuning is read-only until Prism reconnects.".into(),
    }
}

#[test]
fn prism_sends_a_notification_whose_click_only_opens_prism() {
    let desktop = Desktop::start();

    let shown = desktop.show(notification(1, None));

    assert_eq!(shown.app_name, "Prism");
    assert_eq!(shown.replaces_id, 0);
    assert_eq!(shown.app_icon, "io.github.tobytaken.Prism");
    assert_eq!(shown.summary, "Prism lost its connection to prismd");
    assert_eq!(
        shown.body,
        "prismd isn't running. Tuning is read-only until Prism reconnects."
    );
    // Only the default action, which clicking the notification invokes: no write actions.
    assert_eq!(shown.actions, ["default", "Open Prism"]);
    assert_eq!(
        shown.desktop_entry.as_deref(),
        Some("io.github.tobytaken.Prism")
    );
}

#[test]
fn a_click_reaches_prism_with_the_servers_activation_token() {
    let desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    desktop.signal(
        false,
        "ActivationToken",
        &(shown.id, "token-from-the-server"),
    );
    desktop.signal(false, "ActionInvoked", &(shown.id, "default"));

    assert_eq!(
        desktop.event(),
        NotificationEvent::Clicked {
            id: NotificationId(1),
            activation_token: Some("token-from-the-server".into()),
        }
    );
    // The server closes a clicked notification, which Prism already counts as gone.
    desktop.signal(false, "NotificationClosed", &(shown.id, 2u32));
    desktop.no_event();
}

#[test]
fn signals_sent_only_to_prism_reach_it_too() {
    let desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    desktop.signal(true, "ActivationToken", &(shown.id, "token"));
    desktop.signal(true, "ActionInvoked", &(shown.id, "default"));

    assert_eq!(
        desktop.event(),
        NotificationEvent::Clicked {
            id: NotificationId(1),
            activation_token: Some("token".into()),
        }
    );
}

#[test]
fn a_click_without_a_token_still_reaches_prism() {
    let desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    desktop.signal(false, "ActionInvoked", &(shown.id, "default"));

    assert_eq!(
        desktop.event(),
        NotificationEvent::Clicked {
            id: NotificationId(1),
            activation_token: None,
        }
    );
}

#[test]
fn a_replacement_takes_the_server_id_of_the_one_it_replaces() {
    let desktop = Desktop::start();
    let first = desktop.show(notification(1, None));

    let second = desktop.show(notification(2, Some(1)));

    assert_eq!(second.replaces_id, first.id);
    desktop.signal(false, "NotificationClosed", &(second.id, 1u32));
    assert_eq!(
        desktop.event(),
        NotificationEvent::Closed(NotificationId(2))
    );
}

#[test]
fn withdrawing_closes_the_notification_on_the_server() {
    let desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    desktop.notifier().withdraw(NotificationId(1));

    assert!(matches!(desktop.call(), Call::Close(id) if id == shown.id));
    // Prism withdrew it, so it needn't hear that it closed.
    desktop.signal(false, "NotificationClosed", &(shown.id, 3u32));
    desktop.no_event();
}

#[test]
fn other_apps_notifications_are_none_of_prisms_business() {
    let desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    desktop.signal(false, "ActionInvoked", &(shown.id + 1, "default"));
    desktop.signal(false, "NotificationClosed", &(shown.id + 1, 2u32));

    desktop.no_event();
}

#[test]
fn a_click_that_overtakes_the_servers_answer_keeps_its_token() {
    let desktop = Desktop::clicking_at_once();

    desktop.show(notification(1, None));

    assert_eq!(
        desktop.event(),
        NotificationEvent::Clicked {
            id: NotificationId(1),
            activation_token: Some("early".into()),
        }
    );
    desktop.no_event();
}

#[test]
fn withdrawals_queued_as_prism_quits_still_reach_the_server() {
    let mut desktop = Desktop::start();
    let shown = desktop.show(notification(1, None));

    let notifier = desktop.notifier.take().unwrap();
    notifier.withdraw(NotificationId(1));
    drop(notifier);

    assert!(matches!(desktop.calls.try_recv(), Ok(Call::Close(id)) if id == shown.id));
}
