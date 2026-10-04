use std::collections::HashMap;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::framing::{read_frame, write_frame};
use crate::gpu::{GpuCommandError, GpuCommandErrorCode, GpuDaemonStatus};
use crate::protocol::{DaemonEvent, DaemonRequest, DaemonResponse, SessionMessage, SessionRequest};
use once_cell::sync::OnceCell;

pub(super) static SESSION: OnceCell<Arc<SessionTransport>> = OnceCell::new();
type Reply = Result<DaemonResponse, GpuCommandError>;

#[derive(Default)]
struct Connection {
    opening: Option<UnixStream>,
    writer: Option<UnixStream>,
    next_id: u64,
    pending: HashMap<u64, Sender<Reply>>,
}

#[derive(Default)]
pub(super) struct SessionTransport {
    connection: Mutex<Connection>,
    stopping: AtomicBool,
}

impl SessionTransport {
    pub fn request(&self, request: DaemonRequest) -> Reply {
        let (sender, receiver) = mpsc::channel();
        let id = {
            let mut connection = self.connection.lock().unwrap();
            if connection.writer.is_none() {
                return Err(unavailable("Prism daemon session is disconnected"));
            }
            connection.next_id += 1;
            let id = connection.next_id;
            connection.pending.insert(id, sender);
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
        let result = receiver.recv_timeout(Duration::from_secs(2));
        self.connection.lock().unwrap().pending.remove(&id);
        result.unwrap_or_else(|_| {
            Err(unavailable(
                "Timed out waiting for the Prism daemon session",
            ))
        })
    }

    fn disconnect(&self) {
        let mut connection = self.connection.lock().unwrap();
        if let Some(stream) = connection.opening.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(stream) = connection.writer.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        for (_, sender) in connection.pending.drain() {
            let _ = sender.send(Err(unavailable("Prism daemon session closed")));
        }
    }
}

pub struct SessionClient {
    transport: Arc<SessionTransport>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for SessionClient {
    fn drop(&mut self) {
        self.transport.stopping.store(true, Ordering::SeqCst);
        self.transport.disconnect();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// One session for the sidecar's lifetime. Probes and driver requests remain one-shot.
pub fn start_session(notify: impl Fn(GpuDaemonStatus) + Send + 'static) -> SessionClient {
    let transport = Arc::new(SessionTransport::default());
    assert!(
        SESSION.set(Arc::clone(&transport)).is_ok(),
        "a sidecar has only one daemon session"
    );
    let running = Arc::clone(&transport);
    let socket_path = super::socket_path();
    let worker = thread::spawn(move || {
        let mut last_disconnected = None;
        while !running.stopping.load(Ordering::SeqCst) {
            let mut connected = false;
            let result = connect_and_receive(&running, &socket_path, &mut |status| {
                connected = true;
                notify(status);
            });
            running.disconnect();
            if running.stopping.load(Ordering::SeqCst) {
                break;
            }
            if connected {
                last_disconnected = None;
            }
            let status = GpuDaemonStatus::disconnected(
                &socket_path,
                Some(
                    result
                        .err()
                        .unwrap_or_else(|| "Prism daemon session closed".into()),
                ),
            );
            if last_disconnected.as_ref() != Some(&status) {
                notify(status.clone());
                last_disconnected = Some(status);
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
    SessionClient {
        transport,
        worker: Some(worker),
    }
}

fn connect_and_receive(
    transport: &SessionTransport,
    socket: &str,
    publish: &mut impl FnMut(GpuDaemonStatus),
) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket).map_err(|error| error.to_string())?;
    {
        let mut connection = transport.connection.lock().unwrap();
        if transport.stopping.load(Ordering::SeqCst) {
            return Ok(());
        }
        connection.opening = Some(stream.try_clone().map_err(|error| error.to_string())?);
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    write_frame(
        &mut stream,
        &SessionRequest {
            id: 0,
            request: DaemonRequest::OpenSession,
        },
    )
    .map_err(|error| error.to_string())?;
    let mut status =
        match read_frame::<SessionMessage>(&mut stream).map_err(|error| error.to_string())? {
            Some(SessionMessage::Response {
                id: 0,
                response: DaemonResponse::DaemonStatus(status),
            }) => status,
            Some(SessionMessage::Response {
                response: DaemonResponse::Error(error),
                ..
            }) => return Err(error.message),
            _ => return Err("invalid daemon session opening response".into()),
        };
    stream
        .set_read_timeout(None)
        .map_err(|error| error.to_string())?;
    {
        let mut connection = transport.connection.lock().unwrap();
        if transport.stopping.load(Ordering::SeqCst) {
            return Ok(());
        }
        connection.opening = None;
        connection.writer = Some(stream.try_clone().map_err(|error| error.to_string())?);
    }
    publish(status.clone());
    while let Some(message) =
        read_frame::<SessionMessage>(&mut stream).map_err(|error| error.to_string())?
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
                publish(status.clone());
            }
        }
    }
    Ok(())
}

fn unavailable(message: impl Into<String>) -> GpuCommandError {
    GpuCommandError::new(GpuCommandErrorCode::DaemonUnavailable, message)
}
