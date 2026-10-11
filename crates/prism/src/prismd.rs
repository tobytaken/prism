//! Prism's side of prismd: a session thread that connects, reports what the session brings and
//! reconnects whenever there is no session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use prism_protocol::client::{self, ConnectError, Connected, SessionSender};
use prism_protocol::{
    DaemonEvent, DaemonRequest, DaemonSnapshot, HelloRequest, SessionMessage, Welcome,
};
use prism_ui::SessionFailure;

/// How Prism reaches prismd. Production connects to its socket; tests and the simulator wire
/// fakes.
pub trait PrismdConnector {
    /// Starts connecting, and keeps reconnecting whenever there is no session, calling `report`
    /// with each change from any thread until it returns false.
    fn start(&self, report: Box<dyn Fn(PrismdReport) -> bool + Send + Sync>);

    /// Sends `request` on the current session; with no session it's dropped.
    fn send(&self, request: DaemonRequest);
}

/// What the connection to prismd brought.
#[derive(Debug, Clone, PartialEq)]
pub enum PrismdReport {
    /// A session opened on `socket`, starting from `snapshot`.
    Connected {
        welcome: Welcome,
        snapshot: DaemonSnapshot,
        socket: String,
    },
    /// There is no session: an attempt failed, or the session ended.
    NotConnected(SessionFailure),
    /// prismd pushed an event to the session.
    Event(DaemonEvent),
}

/// Connects to prismd's socket on a thread of its own.
pub struct SocketConnector {
    socket: PathBuf,
    /// The current session's sender, while there is a session.
    session: Arc<Mutex<Option<SessionSender>>>,
}

impl SocketConnector {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            session: Arc::default(),
        }
    }

    /// prismd's socket, where `PRISM_SOCKET_PATH` or the installed service puts it.
    pub fn production() -> Self {
        Self::new(client::socket_path())
    }
}

impl PrismdConnector for SocketConnector {
    fn start(&self, report: Box<dyn Fn(PrismdReport) -> bool + Send + Sync>) {
        let report: Arc<dyn Fn(PrismdReport) -> bool + Send + Sync> = Arc::from(report);
        let socket = self.socket.clone();
        let current = Arc::clone(&self.session);
        let reporting = Arc::clone(&report);
        let started = thread::Builder::new()
            .name("prism-prismd".into())
            .spawn(move || run_sessions(&socket, &current, &*reporting));
        if let Err(error) = started {
            eprintln!("prism: can't start the prismd connection: {error}");
            report(PrismdReport::NotConnected(SessionFailure::Unavailable {
                message: format!("Prism couldn't start its prismd connection: {error}"),
            }));
        }
    }

    fn send(&self, request: DaemonRequest) {
        let Some(sender) = self.session.lock().unwrap().clone() else {
            return;
        };
        // A failed write ends the session, which the session thread reports.
        let _ = sender.send(request);
    }
}

/// Opens one session after another until `report` returns false, waiting as the backoff says
/// after each attempt that fails and each session that ends.
fn run_sessions(
    socket: &std::path::Path,
    current: &Mutex<Option<SessionSender>>,
    report: &(dyn Fn(PrismdReport) -> bool + Send + Sync),
) {
    let hello = HelloRequest::current(env!("CARGO_PKG_VERSION"));
    let mut backoff = Backoff::default();
    // Reported once until the reason changes, so retries don't wake the UI thread.
    let mut reported: Option<SessionFailure> = None;
    loop {
        let reason = match client::connect(socket, hello.clone()) {
            Ok(Connected {
                welcome,
                snapshot,
                mut session,
            }) => {
                backoff.reset();
                *current.lock().unwrap() = Some(session.sender());
                let mut open = report(PrismdReport::Connected {
                    welcome,
                    snapshot,
                    socket: socket.display().to_string(),
                });
                while open {
                    open = match session.next_message() {
                        Ok(Some(SessionMessage::Event { event })) => {
                            report(PrismdReport::Event(event))
                        }
                        // Responses only feed what's shown to the user, which lands with the
                        // first commands.
                        Ok(Some(_)) => true,
                        Ok(None) | Err(_) => break,
                    };
                }
                *current.lock().unwrap() = None;
                if !open {
                    return;
                }
                let reason = SessionFailure::Unavailable {
                    message: "The session with prismd ended".into(),
                };
                reported = Some(reason.clone());
                if !report(PrismdReport::NotConnected(reason)) {
                    return;
                }
                // Waits too, so a prismd that keeps ending sessions isn't reconnected in a loop.
                thread::sleep(backoff.next_wait());
                continue;
            }
            Err(ConnectError::Unavailable(error)) => SessionFailure::Unavailable {
                message: error.message,
            },
            Err(ConnectError::Incompatible(incompatible)) => {
                SessionFailure::Incompatible(incompatible)
            }
        };
        if reported.as_ref() != Some(&reason) {
            reported = Some(reason.clone());
            if !report(PrismdReport::NotConnected(reason)) {
                return;
            }
        }
        thread::sleep(backoff.next_wait());
    }
}

/// How long to wait before each new attempt to connect: from 1 s, doubling up to 5 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Backoff {
    next: Duration,
}

impl Backoff {
    const FIRST: Duration = Duration::from_secs(1);
    const CAP: Duration = Duration::from_secs(5);

    /// The wait before the next attempt; each one is longer, up to the cap.
    pub(crate) fn next_wait(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(Self::CAP);
        wait
    }

    /// Starts again from the shortest wait, as after a session.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: Self::FIRST }
    }
}

/// Never connects, such as when Prism runs on simulated GPUs.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPrismd;

impl PrismdConnector for NoPrismd {
    fn start(&self, report: Box<dyn Fn(PrismdReport) -> bool + Send + Sync>) {
        report(PrismdReport::NotConnected(SessionFailure::Unavailable {
            message: "Prism runs on simulated GPUs, without prismd".into(),
        }));
    }

    fn send(&self, _: DaemonRequest) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_from_one_second_to_a_five_second_cap() {
        let mut backoff = Backoff::default();

        let waits: Vec<_> = (0..6).map(|_| backoff.next_wait().as_secs()).collect();

        assert_eq!(waits, [1, 2, 4, 5, 5, 5]);
    }

    #[test]
    fn a_session_starts_the_backoff_again() {
        let mut backoff = Backoff::default();
        backoff.next_wait();
        backoff.next_wait();

        backoff.reset();

        assert_eq!(backoff.next_wait(), Duration::from_secs(1));
    }
}
