//! Desktop notifications through the desktop's `org.freedesktop.Notifications` server (ADR 0012).
//!
//! Prism speaks to the server over zbus itself rather than through notify-rust, which ignores the
//! `ActivationToken` signal a click comes with. Without that token, Wayland desktops such as GNOME
//! won't focus the window the click raises. Sending and the signals share one connection, since
//! some servers only send a notification's signals to whoever sent it.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

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
    worker: Option<JoinHandle<()>>,
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
    /// The server's id for the notification the one waiting replaces. The server may already show
    /// the replacement under it, so until the answer comes, its ending may be either one's.
    replacing: Option<u32>,
    /// Notifications that ended while Prism was waiting, which it didn't know it sent or which may
    /// be the one it's replacing. Any may be the one it's waiting for, since a signal can overtake
    /// the server's answer.
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
    pub fn with_connection(connection: Connection) -> Self {
        Self::connecting(move || Ok(connection))
    }

    fn connecting(connect: impl FnOnce() -> zbus::Result<Connection> + Send + 'static) -> Self {
        let (commands, received) = mpsc::channel();
        let state = Arc::<Mutex<State>>::default();
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("prism-notifications".into())
            .spawn(move || {
                if let Err(error) = serve(connect, &received, &shared) {
                    eprintln!("prism: can't send desktop notifications: {error}");
                }
            })
            .inspect_err(|error| eprintln!("prism: can't send desktop notifications: {error}"))
            .ok();
        Self {
            commands,
            state,
            worker,
        }
    }
}

impl Drop for DesktopNotifier {
    /// Waits for what's queued, such as the withdrawals as Prism quits, so no notification
    /// outlives Prism.
    fn drop(&mut self) {
        // Hanging up ends the worker once it has done the rest.
        self.commands = mpsc::channel().0;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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
    let replaces = sending(state, notification);
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
    answered(state, notification, answer.as_ref().ok().copied());
    answer.map(drop)
}

/// Notes that Prism is sending `notification`, and gives the server's id for the one it
/// replaces, or 0.
fn sending(state: &Mutex<State>, notification: &Notification) -> u32 {
    let mut state = lock(state);
    state.notifying = true;
    state.replacing = notification
        .replaces
        .and_then(|replaced| state.sent.get(&replaced).copied());
    state.replacing.unwrap_or(0)
}

/// Takes in the server's answer to `notification`: the server's id for it, or none if the
/// server didn't show it.
fn answered(state: &Mutex<State>, notification: &Notification, answer: Option<u32>) {
    let mut state = lock(state);
    state.notifying = false;
    state.replacing = None;
    // The one replaced stays Prism's until the server has shown this one in its place.
    if let Some(server_id) = answer {
        if let Some(replaced) = notification.replaces {
            state.sent.remove(&replaced);
        }
        state.sent.insert(notification.id, server_id);
    }
    let early = std::mem::take(&mut state.early);
    let ended: Vec<_> = early
        .iter()
        .filter_map(|&(server_id, ending)| state.end(server_id, ending))
        .collect();
    for (server_id, ending) in early {
        if matches!(ending, Ending::Closed) {
            state.tokens.remove(&server_id);
        }
    }
    tell(state, ended);
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
    let ended = if state.replacing == Some(server_id) {
        None
    } else {
        state.end(server_id, ending)
    };
    if ended.is_none() && state.notifying {
        // Its token waits with it.
        state.early.push((server_id, ending));
    } else if matches!(ending, Ending::Closed) {
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

/// Tells the listener about `events`, letting go of the state first.
fn tell(state: MutexGuard<'_, State>, events: impl IntoIterator<Item = NotificationEvent>) {
    let listener = state.listener.clone();
    drop(state);
    if let Some(listener) = listener {
        events.into_iter().for_each(|event| listener(event));
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(id: u64, replaces: Option<u64>) -> Notification {
        Notification {
            id: NotificationId(id),
            replaces: replaces.map(NotificationId),
            summary: "Prism lost its connection to prismd".into(),
            body: "prismd isn't running. Tuning is read-only until Prism reconnects.".into(),
        }
    }

    /// Prism's notification state, and what its listener hears.
    fn listening() -> (Mutex<State>, Receiver<NotificationEvent>) {
        let (sender, events) = mpsc::channel();
        let state = Mutex::new(State::default());
        lock(&state).listener = Some(Arc::new(move |event| {
            let _ = sender.send(event);
        }));
        (state, events)
    }

    /// Sends `notification`, which the server shows as `server_id`.
    fn show(state: &Mutex<State>, notification: &Notification, server_id: u32) {
        sending(state, notification);
        answered(state, notification, Some(server_id));
    }

    fn closed(server_id: u32) -> Message {
        Message::signal(PATH, INTERFACE, "NotificationClosed")
            .unwrap()
            .build(&(server_id, 2u32))
            .unwrap()
    }

    #[test]
    fn a_closing_that_overtakes_the_answer_to_a_replacement_is_the_replacements() {
        let (state, events) = listening();
        show(&state, &notification(1, None), 7);
        let replacement = notification(2, Some(1));

        // The server shows the replacement under the replaced one's id, and the user closes it
        // before Prism hears the server's answer.
        sending(&state, &replacement);
        take_signal(&state, &closed(7));
        answered(&state, &replacement, Some(7));

        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [NotificationEvent::Closed(NotificationId(2))]
        );
    }

    #[test]
    fn a_closing_as_a_replacement_fails_is_the_replaced_ones() {
        let (state, events) = listening();
        show(&state, &notification(1, None), 7);
        let replacement = notification(2, Some(1));

        sending(&state, &replacement);
        take_signal(&state, &closed(7));
        answered(&state, &replacement, None);

        assert_eq!(
            events.try_iter().collect::<Vec<_>>(),
            [NotificationEvent::Closed(NotificationId(1))]
        );
    }
}
