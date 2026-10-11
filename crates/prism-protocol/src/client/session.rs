use std::io;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::RequestError;
use crate::framing::{read_frame, write_frame};
use crate::{
    DaemonRequest, DaemonResponse, DaemonSnapshot, HelloRequest, Incompatible, SessionMessage,
    SessionRequest, Welcome,
};
use prism_gpu::GpuCommandErrorCode;

/// How long the handshake and each request write may take before prismd counts as unavailable.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// A session prismd welcomed, with the snapshot it starts from.
pub struct Connected {
    pub welcome: Welcome,
    pub snapshot: DaemonSnapshot,
    pub session: Session,
}

/// Why a session couldn't be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// prismd isn't listening, didn't answer, or refused the session.
    Unavailable(RequestError),
    /// prismd speaks a protocol this Prism can't.
    Incompatible(Incompatible),
}

/// An open session: messages prismd sends, read on one thread, and requests sent from any.
pub struct Session {
    reader: UnixStream,
    sender: SessionSender,
}

/// Sends requests on a session, from any thread.
#[derive(Clone)]
pub struct SessionSender {
    writer: Arc<Mutex<Writer>>,
}

struct Writer {
    stream: UnixStream,
    /// The latest request id; the handshake is 0.
    last_id: u64,
}

/// Connects to prismd at `socket`, says hello, and waits for its welcome and snapshot.
pub fn connect(socket: &Path, hello: HelloRequest) -> Result<Connected, ConnectError> {
    let unavailable = |error: io::Error| ConnectError::Unavailable(super::map_io_error(error));
    let mut stream = UnixStream::connect(socket).map_err(unavailable)?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(unavailable)?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(unavailable)?;
    write_frame(
        &mut stream,
        &SessionRequest {
            id: 0,
            request: DaemonRequest::Hello(hello.clone()),
        },
    )
    .map_err(unavailable)?;
    let welcome = match read_message(&mut stream).map_err(unavailable)? {
        Some(SessionMessage::Response {
            id: 0,
            response: DaemonResponse::Welcome(welcome),
        }) => welcome,
        Some(SessionMessage::Response {
            id: 0,
            response: DaemonResponse::Incompatible(incompatible),
        }) => return Err(ConnectError::Incompatible(incompatible)),
        Some(SessionMessage::Response {
            id: 0,
            response: DaemonResponse::Error(error),
        }) => return Err(ConnectError::Unavailable(error.into())),
        _ => return Err(invalid("prismd didn't answer the handshake")),
    };
    // Checked on this side too, so a daemon that welcomes anyone still isn't used.
    if !hello.protocol.is_compatible_with(welcome.protocol) {
        return Err(ConnectError::Incompatible(Incompatible {
            protocol: welcome.protocol,
            prismd_version: welcome.prismd_version,
        }));
    }
    let snapshot = match read_message(&mut stream).map_err(unavailable)? {
        Some(SessionMessage::Snapshot { snapshot }) => snapshot,
        _ => return Err(invalid("prismd sent no snapshot after its welcome")),
    };
    // From here on, reads wait for as long as prismd has nothing to say.
    stream.set_read_timeout(None).map_err(unavailable)?;
    let writer = stream.try_clone().map_err(unavailable)?;
    Ok(Connected {
        welcome,
        snapshot,
        session: Session {
            reader: stream,
            sender: SessionSender {
                writer: Arc::new(Mutex::new(Writer {
                    stream: writer,
                    last_id: 0,
                })),
            },
        },
    })
}

impl Session {
    /// Waits for prismd's next message; none once the session has ended. Events this Prism
    /// doesn't know, such as those from a newer prismd, are skipped.
    pub fn next_message(&mut self) -> io::Result<Option<SessionMessage>> {
        read_message(&mut self.reader)
    }

    pub fn sender(&self) -> SessionSender {
        self.sender.clone()
    }
}

impl Drop for Session {
    /// Ends the session, so prismd releases what it held for it.
    fn drop(&mut self) {
        let _ = self.reader.shutdown(Shutdown::Both);
    }
}

impl SessionSender {
    /// Sends `request` and returns the id its response will carry.
    pub fn send(&self, request: DaemonRequest) -> io::Result<u64> {
        let mut writer = self.writer.lock().unwrap();
        writer.last_id += 1;
        let id = writer.last_id;
        write_frame(&mut writer.stream, &SessionRequest { id, request })?;
        Ok(id)
    }
}

/// The next message, skipping events this side doesn't know.
fn read_message(stream: &mut UnixStream) -> io::Result<Option<SessionMessage>> {
    loop {
        let Some(frame) = read_frame::<serde_json::Value>(stream)? else {
            return Ok(None);
        };
        let is_event = frame.get("type").and_then(serde_json::Value::as_str) == Some("event");
        match serde_json::from_value(frame) {
            Ok(message) => return Ok(Some(message)),
            Err(_) if is_event => continue,
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

fn invalid(message: &str) -> ConnectError {
    ConnectError::Unavailable(RequestError::new(GpuCommandErrorCode::Internal, message))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::thread;

    use super::*;
    use crate::{DaemonEvent, ProtocolVersion};

    fn contract(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/contracts/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// prismd on a socket of its own that answers the hello with `answer` and then sends
    /// `messages`, one frame each.
    fn prismd(
        answer: serde_json::Value,
        messages: Vec<serde_json::Value>,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("prismd.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello: SessionRequest = read_frame(&mut stream).unwrap().unwrap();
            write_frame(&mut stream, &answer).unwrap();
            for message in messages {
                write_frame(&mut stream, &message).unwrap();
            }
        });
        (dir, socket)
    }

    #[test]
    fn a_session_starts_from_the_snapshot_and_skips_events_it_does_not_know() {
        let handshake = contract("session-handshake.json");
        let store_health = contract("session-store-health.json");
        let (_dir, socket) = prismd(
            handshake["welcome"].clone(),
            vec![
                contract("daemon-snapshot.json"),
                serde_json::json!({ "type": "event", "event": { "type": "from_a_newer_prismd", "payload": {} } }),
                store_health,
            ],
        );

        let Connected {
            welcome,
            snapshot,
            mut session,
        } = connect(&socket, HelloRequest::current("0.1.0")).unwrap();

        assert_eq!(welcome.protocol, ProtocolVersion::CURRENT);
        assert_eq!(snapshot.inventory.gpus.len(), 1);
        assert_eq!(
            session.next_message().unwrap(),
            Some(SessionMessage::Event {
                event: DaemonEvent::StoreHealthChanged(crate::TuningStoreHealth::Healthy {})
            })
        );
        assert_eq!(session.next_message().unwrap(), None);
    }

    #[test]
    fn an_incompatible_prismd_is_not_used() {
        let handshake = contract("session-handshake.json");
        let (_dir, socket) = prismd(handshake["incompatible"].clone(), vec![]);

        let refused = connect(
            &socket,
            HelloRequest {
                protocol: ProtocolVersion { major: 0, minor: 2 },
                prism_version: "0.2.0".into(),
            },
        );

        assert!(matches!(
            refused,
            Err(ConnectError::Incompatible(Incompatible { protocol, .. }))
                if protocol == ProtocolVersion::CURRENT
        ));
    }

    #[test]
    fn a_missing_socket_means_prismd_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();

        let refused = connect(
            &dir.path().join("prismd.sock"),
            HelloRequest::current("0.1.0"),
        );

        assert!(matches!(
            refused,
            Err(ConnectError::Unavailable(RequestError {
                code: super::super::RequestErrorCode::DaemonUnavailable,
                ..
            }))
        ));
    }
}
