//! The real prismd connector against a scripted prismd on a socket: it reports when there is no
//! session, and connects by itself once prismd answers.

use std::os::unix::net::UnixListener;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use prism::{PrismdConnector, PrismdReport, SocketConnector};
use prism_protocol::framing::{read_frame, write_frame};
use prism_protocol::{DaemonEvent, DaemonRequest, SessionRequest, TuningStoreHealth};
use prism_ui::SessionFailure;

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

/// Serves one session: welcomes the hello, sends the snapshot and a store health event, waits for
/// one request, and returns it once the session is closed.
fn serve_one_session(listener: &UnixListener) -> DaemonRequest {
    let (mut stream, _) = listener.accept().unwrap();
    let hello: SessionRequest = read_frame(&mut stream).unwrap().unwrap();
    assert!(matches!(hello.request, DaemonRequest::Hello(_)));
    write_frame(&mut stream, &contract("session-handshake.json")["welcome"]).unwrap();
    write_frame(&mut stream, &contract("daemon-snapshot.json")).unwrap();
    write_frame(&mut stream, &contract("session-store-health.json")).unwrap();
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
