//! Desktop notifications through the desktop's `org.freedesktop.Notifications` server (ADR 0012).
//!
//! Prism speaks to the server over zbus itself rather than through notify-rust, which ignores the
//! `ActivationToken` signal a click comes with. Without that token, Wayland desktops such as GNOME
//! won't focus the window the click raises. Sending and the signals share one connection, since
//! some servers only send a notification's signals to whoever sent it.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use prism_ui::{Notification, NotificationId};
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::Value;
use zbus::{MatchRule, Message};

use crate::single_instance::APP_ID;

/// Sends desktop notifications and tells what happens to them.
pub trait Notifier {
    /// Calls `event` with what happens to the notifications sent, from any thread.
    fn listen(&self, event: Box<dyn Fn(NotificationEvent) + Send + Sync>);

    /// Sends `notification`, in place of the one it replaces.
    fn notify(&self, notification: Notification);

    /// Closes the notification, if it's still showing.
    fn withdraw(&self, id: NotificationId);
}

/// What happened to a notification Prism sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    /// The user clicked it, and the server handed over this activation token, if any.
    Clicked {
        id: NotificationId,
        activation_token: Option<String>,
    },
    /// The user dismissed it, or it expired.
    Closed(NotificationId),
}

const BUS_NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
/// The action a click invokes. It only opens Prism: notifications have no write actions.
const DEFAULT_ACTION: &str = "default";

/// Notifications through the desktop's notification server, on a thread of their own, since a
/// server can take its time to answer.
pub struct DesktopNotifier {
    commands: Sender<Command>,
    state: Arc<Mutex<State>>,
}

enum Command {
    Notify(Notification),
    Withdraw(NotificationId),
}

#[derive(Default)]
struct State {
    listener: Option<Arc<dyn Fn(NotificationEvent) + Send + Sync>>,
    /// The server's id for each live notification Prism sent.
    sent: HashMap<NotificationId, u32>,
    /// The activation token the server sent ahead of a click, by the server's id.
    tokens: HashMap<u32, String>,
    /// Whether a `Notify` call is waiting for the server's answer.
    notifying: bool,
    /// Notifications Prism didn't know it sent that ended while it was waiting, which may be the
    /// one it's waiting for, since a signal can overtake the server's answer.
    early: Vec<(u32, Ending)>,
}

/// How a notification ended.
#[derive(Debug, Clone, Copy)]
enum Ending {
    Clicked,
    Closed,
}

impl DesktopNotifier {
    /// On the session bus. Without one, notifications go nowhere.
    pub fn session() -> Self {
        Self::connecting(Connection::session)
    }

    /// On `connection`, a connection to the session bus.
    pub fn on(connection: Connection) -> Self {
        Self::connecting(move || Ok(connection))
    }

    fn connecting(connect: impl FnOnce() -> zbus::Result<Connection> + Send + 'static) -> Self {
        let (commands, received) = mpsc::channel();
        let state = Arc::<Mutex<State>>::default();
        let shared = state.clone();
        let started = thread::Builder::new()
            .name("prism-notifications".into())
            .spawn(move || {
                if let Err(error) = serve(connect, &received, &shared) {
                    eprintln!("prism: can't send desktop notifications: {error}");
                }
            });
        if let Err(error) = started {
            eprintln!("prism: can't send desktop notifications: {error}");
        }
        Self { commands, state }
    }
}

impl Notifier for DesktopNotifier {
    fn listen(&self, event: Box<dyn Fn(NotificationEvent) + Send + Sync>) {
        lock(&self.state).listener = Some(Arc::from(event));
    }

    fn notify(&self, notification: Notification) {
        let _ = self.commands.send(Command::Notify(notification));
    }

    fn withdraw(&self, id: NotificationId) {
        let _ = self.commands.send(Command::Withdraw(id));
    }
}

/// Connects, listens for the server's signals on a thread of its own, and performs commands until
/// Prism drops the notifier.
fn serve(
    connect: impl FnOnce() -> zbus::Result<Connection>,
    commands: &Receiver<Command>,
    state: &Arc<Mutex<State>>,
) -> zbus::Result<()> {
    let connection = connect()?;
    // Matched before anything is sent, so no click is missed.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(INTERFACE)?
        .path(PATH)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, &connection, None)?;
    let shared = state.clone();
    thread::Builder::new()
        .name("prism-notification-signals".into())
        .spawn(move || {
            for message in signals.flatten() {
                take_signal(&shared, &message);
            }
        })
        .map_err(|error| zbus::Error::Failure(error.to_string()))?;
    for command in commands {
        let done = match command {
            Command::Notify(notification) => notify(&connection, state, &notification),
            Command::Withdraw(id) => withdraw(&connection, state, id),
        };
        if let Err(error) = done {
            eprintln!("prism: desktop notification failed: {error}");
        }
    }
    Ok(())
}

fn notify(
    connection: &Connection,
    state: &Mutex<State>,
    notification: &Notification,
) -> zbus::Result<()> {
    let replaces = {
        let mut state = lock(state);
        state.notifying = true;
        notification
            .replaces
            .and_then(|replaced| state.sent.remove(&replaced))
            .unwrap_or(0)
    };
    let hints = HashMap::from([("desktop-entry", Value::from(APP_ID))]);
    let answer = connection
        .call_method(
            Some(BUS_NAME),
            PATH,
            Some(INTERFACE),
            "Notify",
            &(
                "Prism",
                replaces,
                APP_ID,
                &notification.summary,
                &notification.body,
                &[DEFAULT_ACTION, "Open Prism"][..],
                hints,
                // The server's own timeout.
                -1i32,
            ),
        )
        .and_then(|reply| reply.body().deserialize::<u32>());
    let mut state = lock(state);
    state.notifying = false;
    let early = std::mem::take(&mut state.early);
    let server_id = answer?;
    state.sent.insert(notification.id, server_id);
    let ended = early
        .into_iter()
        .find(|(early, _)| *early == server_id)
        .and_then(|(_, ending)| state.end(server_id, ending));
    tell(state, ended);
    Ok(())
}

fn withdraw(connection: &Connection, state: &Mutex<State>, id: NotificationId) -> zbus::Result<()> {
    let Some(server_id) = lock(state).sent.remove(&id) else {
        return Ok(());
    };
    connection.call_method(
        Some(BUS_NAME),
        PATH,
        Some(INTERFACE),
        "CloseNotification",
        &(server_id,),
    )?;
    Ok(())
}

/// Takes in one of the server's signals, telling the listener what happened to a notification
/// Prism sent.
fn take_signal(state: &Mutex<State>, message: &Message) {
    let header = message.header();
    let body = message.body();
    let (server_id, ending) = match header.member().map(|member| member.as_str()) {
        Some("ActivationToken") => {
            // Kept even for a notification Prism doesn't know it sent yet. Closing drops it,
            // whoever sent the notification.
            if let Ok((server_id, token)) = body.deserialize::<(u32, String)>() {
                lock(state).tokens.insert(server_id, token);
            }
            return;
        }
        Some("ActionInvoked") => match body.deserialize::<(u32, String)>() {
            Ok((server_id, action)) if action == DEFAULT_ACTION => (server_id, Ending::Clicked),
            _ => return,
        },
        Some("NotificationClosed") => match body.deserialize::<(u32, u32)>() {
            Ok((server_id, _reason)) => (server_id, Ending::Closed),
            _ => return,
        },
        _ => return,
    };
    let mut state = lock(state);
    let ended = state.end(server_id, ending);
    if ended.is_none() && state.notifying {
        state.early.push((server_id, ending));
    }
    if matches!(ending, Ending::Closed) {
        state.tokens.remove(&server_id);
    }
    tell(state, ended);
}

impl State {
    /// What ending the server's `server_id` means for Prism, if Prism sent it, which is then no
    /// longer live.
    fn end(&mut self, server_id: u32, ending: Ending) -> Option<NotificationEvent> {
        let id = self
            .sent
            .iter()
            .find_map(|(id, sent)| (*sent == server_id).then_some(*id))?;
        self.sent.remove(&id);
        Some(match ending {
            Ending::Clicked => NotificationEvent::Clicked {
                id,
                activation_token: self.tokens.remove(&server_id),
            },
            Ending::Closed => NotificationEvent::Closed(id),
        })
    }
}

/// Tells the listener about `event`, if any, letting go of the state first.
fn tell(state: MutexGuard<'_, State>, event: Option<NotificationEvent>) {
    let listener = state.listener.clone();
    drop(state);
    if let (Some(listener), Some(event)) = (listener, event) {
        listener(event);
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
