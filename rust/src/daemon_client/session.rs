use std::collections::HashMap;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::framing::{read_frame, write_frame};
use crate::gpu::{GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus};
use crate::protocol::{
    DaemonEvent, DaemonRequest, DaemonResponse, GpuTuningState, SessionMessage, SessionRequest,
};
use once_cell::sync::OnceCell;

pub(super) static SESSION: OnceCell<Arc<SessionTransport>> = OnceCell::new();
type Reply = Result<DaemonResponse, GpuCommandError>;

#[derive(Default)]
struct Connection {
    opening: Option<UnixStream>,
    writer: Option<UnixStream>,
    next_id: u64,
    pending: HashMap<u64, Sender<Reply>>,
    failure: Option<GpuCommandError>,
    edit_holds: Option<crate::protocol::SetEditHoldsRequest>,
}

#[derive(Default)]
pub(super) struct SessionTransport {
    connection: Mutex<Connection>,
    connected: Condvar,
    stopping: AtomicBool,
}

impl SessionTransport {
    pub fn request(&self, request: DaemonRequest) -> Reply {
        if let DaemonRequest::SetEditHolds(holds) = &request {
            // Retain the latest renderer intent even while disconnected.
            self.connection.lock().unwrap().edit_holds = Some(holds.clone());
        }
        let deadline = Instant::now() + super::request_timeout(&request);
        let (sender, receiver) = mpsc::channel();
        let id = {
            let (mut connection, _) = self
                .connected
                .wait_timeout_while(
                    self.connection.lock().unwrap(),
                    Duration::from_secs(2),
                    |connection| {
                        connection.writer.is_none()
                            && !self.stopping.load(Ordering::SeqCst)
                            && !matches!(
                                connection.failure.as_ref().map(|error| &error.code),
                                Some(GpuCommandErrorCode::NoPermission)
                            )
                    },
                )
                .unwrap();
            if connection.writer.is_none() {
                return Err(connection
                    .failure
                    .clone()
                    .unwrap_or_else(|| unavailable("Prism daemon session is disconnected")));
            }
            connection.next_id += 1;
            let id = connection.next_id;
            connection.pending.insert(id, sender);
            // A hold update waiting for reconnection must not overwrite a newer cached set.
            let request = if matches!(request, DaemonRequest::SetEditHolds(_)) {
                DaemonRequest::SetEditHolds(connection.edit_holds.clone().unwrap())
            } else {
                request
            };
            if let Err(error) = write_frame(
                connection.writer.as_mut().unwrap(),
                &SessionRequest { id, request },
            ) {
                connection.pending.remove(&id);
                let _ = connection.writer.as_ref().unwrap().shutdown(Shutdown::Both);
                return Err(super::map_io_error(error));
            }
            id
        };
        let result = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        self.connection.lock().unwrap().pending.remove(&id);
        result.unwrap_or_else(|_| {
            Err(unavailable(
                "Timed out waiting for the Prism daemon session",
            ))
        })
    }

    fn disconnect(&self, reason: GpuCommandError) {
        let mut connection = self.connection.lock().unwrap();
        if let Some(stream) = connection.opening.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(stream) = connection.writer.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        for (_, sender) in connection.pending.drain() {
            let _ = sender.send(Err(reason.clone()));
        }
        connection.failure = Some(reason);
        self.connected.notify_all();
    }
}

pub struct SessionClient {
    transport: Arc<SessionTransport>,
}

impl Drop for SessionClient {
    fn drop(&mut self) {
        self.transport.stopping.store(true, Ordering::SeqCst);
        self.transport
            .disconnect(unavailable("Prism daemon session closed"));
        // The notification thread may be blocked on Electron's stdout pipe. Transport shutdown
        // must not wait for it: returning from sidecar main ends every remaining thread.
    }
}

/// What the session reports without being asked.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionNotification {
    DaemonStatus(GpuDaemonStatus),
    TuningState(GpuTuningState),
    RunningGames(crate::protocol::RunningGamesState),
    SwitchingStatus(crate::protocol::AutomaticSwitchingStatus),
}

/// One session for the sidecar's lifetime. Probes and driver requests remain one-shot.
pub fn start_session(notify: impl Fn(SessionNotification) + Send + 'static) -> SessionClient {
    let transport = Arc::new(SessionTransport::default());
    assert!(
        SESSION.set(Arc::clone(&transport)).is_ok(),
        "a sidecar has only one daemon session"
    );
    let running = Arc::clone(&transport);
    let socket_path = super::socket_path();
    thread::spawn(move || {
        let mut last_disconnected = None;
        while !running.stopping.load(Ordering::SeqCst) {
            let mut connected = false;
            let result = connect_and_receive(&running, &socket_path, &mut |notification| {
                connected = true;
                notify(notification);
            });
            let error = result
                .err()
                .unwrap_or_else(|| unavailable("Prism daemon session closed"));
            running.disconnect(error.clone());
            if running.stopping.load(Ordering::SeqCst) {
                break;
            }
            if connected {
                last_disconnected = None;
            }
            let status = GpuDaemonStatus::disconnected(&socket_path, Some(error.message));
            if last_disconnected.as_ref() != Some(&status) {
                notify(SessionNotification::DaemonStatus(status.clone()));
                last_disconnected = Some(status);
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
    SessionClient { transport }
}

fn connect_and_receive(
    transport: &SessionTransport,
    socket: &str,
    publish: &mut impl FnMut(SessionNotification),
) -> Result<(), GpuCommandError> {
    let mut stream = UnixStream::connect(socket).map_err(super::map_io_error)?;
    {
        let mut connection = transport.connection.lock().unwrap();
        if transport.stopping.load(Ordering::SeqCst) {
            return Ok(());
        }
        connection.opening = Some(stream.try_clone().map_err(super::map_io_error)?);
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(super::map_io_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(super::map_io_error)?;
    write_frame(
        &mut stream,
        &SessionRequest {
            id: 0,
            request: DaemonRequest::OpenSession,
        },
    )
    .map_err(super::map_io_error)?;
    let mut status = match read_frame::<SessionMessage>(&mut stream).map_err(super::map_io_error)? {
        Some(SessionMessage::Response {
            id: 0,
            response: DaemonResponse::DaemonStatus(status),
        }) => status,
        Some(SessionMessage::Response {
            response: DaemonResponse::Error(error),
            ..
        }) => return Err(error),
        _ => {
            return Err(GpuCommandError::new(
                GpuCommandErrorCode::Internal,
                "invalid daemon session opening response",
            ))
        }
    };
    stream.set_read_timeout(None).map_err(super::map_io_error)?;
    {
        let mut connection = transport.connection.lock().unwrap();
        if transport.stopping.load(Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(holds) = connection.edit_holds.clone() {
            connection.next_id += 1;
            write_frame(
                &mut stream,
                &SessionRequest {
                    id: connection.next_id,
                    request: DaemonRequest::SetEditHolds(holds),
                },
            )
            .map_err(super::map_io_error)?;
        }
        connection.opening = None;
        connection.writer = Some(stream.try_clone().map_err(super::map_io_error)?);
        connection.failure = None;
        transport.connected.notify_all();
    }
    publish(SessionNotification::DaemonStatus(status.clone()));
    while let Some(message) =
        read_frame::<SessionMessage>(&mut stream).map_err(super::map_io_error)?
    {
        match message {
            SessionMessage::Response { id, response } => {
                if let Some(sender) = transport.connection.lock().unwrap().pending.remove(&id) {
                    let _ = sender.send(Ok(response));
                }
            }
            SessionMessage::Event {
                event: DaemonEvent::StoreHealthChanged(health),
            } => {
                status.tuning_store = Some(health);
                publish(SessionNotification::DaemonStatus(status.clone()));
            }
            SessionMessage::Event {
                event: DaemonEvent::TuningStateChanged(state),
            } => publish(SessionNotification::TuningState(state)),
            SessionMessage::Event {
                event: DaemonEvent::RunningGamesChanged(state),
            } => publish(SessionNotification::RunningGames(state)),
            SessionMessage::Event {
                event: DaemonEvent::AutomaticSwitchingChanged(status),
            } => publish(SessionNotification::SwitchingStatus(status)),
        }
    }
    Ok(())
}

fn unavailable(message: impl Into<String>) -> GpuCommandError {
    GpuCommandError::new(GpuCommandErrorCode::DaemonUnavailable, message)
}
