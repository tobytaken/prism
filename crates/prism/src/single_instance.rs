//! One Prism per session, through the D-Bus name `io.github.tobytaken.Prism` (ADR 0012).
//!
//! The first Prism owns the name and serves `org.freedesktop.Application` on it. A later launch
//! finds the name taken, calls `Activate` with its activation token, and exits; the first Prism
//! then raises its window with that token. A launch that reaches the first Prism as it quits
//! claims the name instead.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use zbus::blocking::{Connection, Proxy};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::zvariant::{OwnedValue, Value};

use crate::environment::Launches;

/// Prism's app ID, which is also its D-Bus name.
pub const APP_ID: &str = "io.github.tobytaken.Prism";
/// Where the app ID's object lives, following `org.freedesktop.Application`.
const OBJECT_PATH: &str = "/io/github/tobytaken/Prism";
const INTERFACE: &str = "org.freedesktop.Application";
/// The platform data keys an activation token travels under: Wayland's, then X11's startup ID.
const TOKEN_KEYS: [&str; 2] = ["activation-token", "desktop-startup-id"];

/// What claiming the app ID's name found.
pub enum Claim {
    /// This is the only Prism; later launches reach it through these.
    First(BusLaunches),
    /// Another Prism was running and took over this launch, which should exit.
    HandedOver,
}

/// The activation token this process was launched with: Wayland's `XDG_ACTIVATION_TOKEN`, or
/// X11's `DESKTOP_STARTUP_ID`.
pub fn launch_activation_token() -> Option<String> {
    ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .into_iter()
        .find_map(|variable| std::env::var(variable).ok())
        .filter(|token| !token.is_empty())
}

/// Claims the app ID's name on the session bus, or hands this launch and its activation token
/// over to the Prism that owns it.
pub fn claim(activation_token: Option<String>) -> zbus::Result<Claim> {
    claim_on(Connection::session()?, activation_token)
}

/// [`claim`] on a given bus connection.
pub fn claim_on(connection: Connection, activation_token: Option<String>) -> zbus::Result<Claim> {
    let listener = Arc::new(Mutex::new(Listener::Waiting(Vec::new())));
    // Serve first, so a later launch never finds the name without the object.
    connection.object_server().at(
        OBJECT_PATH,
        Application {
            listener: listener.clone(),
        },
    )?;
    // A Prism that quits as this launch reaches it has given up the name by the time the bus tells
    // this launch it won't answer, so this launch claims the name once more.
    let mut claimed_again = false;
    loop {
        match connection.request_name_with_flags(APP_ID, RequestNameFlags::DoNotQueue.into()) {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
                return Ok(Claim::First(BusLaunches {
                    _connection: connection,
                    listener,
                }));
            }
            // zbus reports the name being taken as an error.
            Ok(RequestNameReply::Exists | RequestNameReply::InQueue)
            | Err(zbus::Error::NameTaken) => {
                match activate(&connection, activation_token.as_deref()) {
                    Ok(()) => return Ok(Claim::HandedOver),
                    Err(error) if !claimed_again && owner_quit(&error) => claimed_again = true,
                    Err(error) => {
                        return Err(zbus::Error::Failure(format!(
                            "another Prism owns {APP_ID} but didn't take over this launch: {error}"
                        )));
                    }
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Hands this launch and its activation token over to the Prism that owns the app ID's name.
fn activate(connection: &Connection, activation_token: Option<&str>) -> zbus::Result<()> {
    let proxy = Proxy::new(connection, APP_ID, OBJECT_PATH, INTERFACE)?;
    let platform_data: HashMap<&str, Value<'_>> = activation_token
        .map(|token| (TOKEN_KEYS[0], Value::from(token)))
        .into_iter()
        .collect();
    proxy.call_method("Activate", &(platform_data,))?;
    Ok(())
}

/// Whether `error` says the Prism that owned the name quit before it answered.
fn owner_quit(error: &zbus::Error) -> bool {
    matches!(
        zbus::fdo::Error::from(error.clone()),
        zbus::fdo::Error::NoReply(_)
            | zbus::fdo::Error::ServiceUnknown(_)
            | zbus::fdo::Error::NameHasNoOwner(_)
    )
}

/// Later launches that reach the first Prism over the session bus. Keeps the name while it
/// lives.
pub struct BusLaunches {
    _connection: Connection,
    listener: Arc<Mutex<Listener>>,
}

impl Launches for BusLaunches {
    fn listen(&self, launched: Box<dyn Fn(Option<String>) + Send + Sync>) {
        let mut listener = self.listener.lock().unwrap_or_else(PoisonError::into_inner);
        if let Listener::Waiting(early) = &mut *listener {
            for token in early.drain(..) {
                launched(token);
            }
        }
        *listener = Listener::Listening(launched);
    }
}

/// Who hears of later launches.
enum Listener {
    /// Nobody yet, so launches wait here, with their activation tokens.
    Waiting(Vec<Option<String>>),
    Listening(Box<dyn Fn(Option<String>) + Send + Sync>),
}

/// `org.freedesktop.Application` on the app ID's object.
struct Application {
    listener: Arc<Mutex<Listener>>,
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        let token = TOKEN_KEYS
            .into_iter()
            .filter_map(|key| platform_data.get(key))
            .find_map(|token| String::try_from(token.clone()).ok());
        match &mut *self.listener.lock().unwrap_or_else(PoisonError::into_inner) {
            Listener::Waiting(early) => early.push(token),
            Listener::Listening(launched) => launched(token),
        }
    }
}
