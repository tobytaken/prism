//! The real prismd connector against a scripted prismd on a socket: it reports when there is no
//! session, and connects by itself once prismd answers.

use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use prism::{PrismdConnector, PrismdReport, SocketConnector};
use prism_protocol::framing::{MAX_FRAME_BYTES, read_frame, write_frame};
use prism_protocol::{
    DaemonEvent, DaemonRequest, RenameProfileRequest, SessionRequest, TuningStoreHealth,
};
use prism_ui::{CommandId, SessionFailure};

/// Longer than the longest backoff, so a missed reconnect fails instead of hanging.
const DEADLINE: Duration = Duration::from_secs(8);

fn contract(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/../prism-protocol/tests/contracts/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn start(connector: &SocketConnector) -> Receiver<PrismdReport> {
    let (sender, reports) = mpsc::channel();
    connector.start(Box::new(move |report| sender.send(report).is_ok()));
    reports
}

fn next(reports: &Receiver<PrismdReport>) -> PrismdReport {
    reports
        .recv_timeout(DEADLINE)
        .expect("the connector reported nothing")
}

/// Opens one session: welcomes the hello, then sends the snapshot and a store health event.
fn open_session(listener: &UnixListener) -> UnixStream {
    let (mut stream, _) = listener.accept().unwrap();
    let hello: SessionRequest = read_frame(&mut stream).unwrap().unwrap();
    assert!(matches!(hello.request, DaemonRequest::Hello(_)));
    write_frame(&mut stream, &contract("session-handshake.json")["welcome"]).unwrap();
    write_frame(&mut stream, &contract("daemon-snapshot.json")).unwrap();
    write_frame(&mut stream, &contract("session-store-health.json")).unwrap();
    stream
}

/// Serves one session, waits for one request, and returns it once the session is closed.
fn serve_one_session(listener: &UnixListener) -> DaemonRequest {
    let mut stream = open_session(listener);
    let request: SessionRequest = read_frame(&mut stream).unwrap().unwrap();
    request.request
}

#[test]
fn the_connector_reports_no_session_then_connects_once_prismd_answers_and_after_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("prismd.sock");
    let connector = SocketConnector::new(&socket);

    let reports = start(&connector);

    assert!(matches!(
        next(&reports),
        PrismdReport::NotConnected(SessionFailure::Unavailable { .. })
    ));

    // prismd starts while the connector waits to retry.
    let listener = UnixListener::bind(&socket).unwrap();
    let serving = std::thread::spawn(move || {
        let first = serve_one_session(&listener);
        let second = serve_one_session(&listener);
        (first, second)
    });
    let PrismdReport::Connected { snapshot, .. } = next(&reports) else {
        panic!("the connector didn't connect")
    };
    assert_eq!(snapshot.inventory.gpus.len(), 1);
    assert_eq!(
        next(&reports),
        PrismdReport::Event(DaemonEvent::StoreHealthChanged(
            TuningStoreHealth::Healthy {}
        ))
    );
    connector.send(DaemonRequest::SubscribeTelemetry);

    // prismd ends the session, and the connector opens a new one by itself.
    assert!(matches!(next(&reports), PrismdReport::NotConnected(_)));
    assert!(matches!(next(&reports), PrismdReport::Connected { .. }));
    next(&reports);
    connector.send(DaemonRequest::SubscribeTelemetry);
    assert_eq!(
        serving.join().unwrap(),
        (
            DaemonRequest::SubscribeTelemetry,
            DaemonRequest::SubscribeTelemetry
        )
    );
}

#[test]
fn a_command_that_cant_be_sent_ends_the_session_so_nothing_waits_for_its_answer() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("prismd.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let serving = std::thread::spawn(move || {
        let mut first = open_session(&listener);
        // Nothing of the command arrives before the session ends.
        assert!(read_frame::<SessionRequest>(&mut first).unwrap().is_none());
        serve_one_session(&listener)
    });
    let connector = SocketConnector::new(&socket);
    let reports = start(&connector);
    assert!(matches!(next(&reports), PrismdReport::Connected { .. }));
    next(&reports);

    // Too large for a frame, so it's refused before any of it is written.
    connector.command(
        CommandId(1),
        DaemonRequest::RenameProfile(RenameProfileRequest {
            profile_id: "quiet".into(),
            name: "x".repeat(MAX_FRAME_BYTES),
        }),
    );

    // The lost session is what tells the UI the command won't be answered.
    assert!(matches!(
        next(&reports),
        PrismdReport::NotConnected(SessionFailure::Unavailable { .. })
    ));
    assert!(matches!(next(&reports), PrismdReport::Connected { .. }));
    next(&reports);
    connector.send(DaemonRequest::SubscribeTelemetry);
    assert_eq!(serving.join().unwrap(), DaemonRequest::SubscribeTelemetry);
}
