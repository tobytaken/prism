//! The fake prismd connection: the same connector interface as the real one, driven by the test
//! on its own thread, with snapshots and events from the shared contract fixtures.

use std::sync::{Arc, Mutex};

use prism::{PrismdConnector, PrismdReport};
use prism_gpu::GpuPerformanceResponse;
use prism_protocol::{DaemonEvent, DaemonRequest, DaemonSnapshot, SessionMessage};
use prism_ui::NotConnected;

/// What the fake daemon is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    NotRunning,
    /// Running, and welcoming sessions.
    Running,
    /// Running, and welcoming sessions read-only: the user isn't in the admin group.
    ReadOnly,
    /// Running, but speaking a protocol this Prism can't.
    Incompatible,
}

/// prismd as Prism sees it through its connector. Clones share the same daemon.
#[derive(Clone)]
pub struct FakePrismd(Arc<Mutex<Fake>>);

type Report = Arc<dyn Fn(PrismdReport) -> bool + Send + Sync>;

struct Fake {
    state: DaemonState,
    /// Where reports go once Prism has started connecting.
    report: Option<Report>,
    requests: Vec<DaemonRequest>,
}

impl FakePrismd {
    pub fn new(state: DaemonState) -> Self {
        Self(Arc::new(Mutex::new(Fake {
            state,
            report: None,
            requests: Vec::new(),
        })))
    }

    /// The side Prism holds.
    pub fn connector(&self) -> Box<dyn PrismdConnector> {
        Box::new(self.clone())
    }

    /// Starts the daemon, which a connecting Prism reaches at once.
    pub fn start_daemon(&self) {
        self.set_state(DaemonState::Running);
    }

    /// Stops the daemon, ending any session.
    pub fn stop_daemon(&self) {
        self.set_state(DaemonState::NotRunning);
    }

    /// Replaces the daemon with one speaking another protocol.
    pub fn make_incompatible(&self) {
        self.set_state(DaemonState::Incompatible);
    }

    /// Pushes `event` to the session.
    ///
    /// # Panics
    ///
    /// Without a session.
    pub fn push(&self, event: DaemonEvent) {
        let fake = self.0.lock().unwrap();
        assert!(fake.state.welcomes(), "no session to push to");
        let report = fake.report.clone().expect("Prism hasn't connected");
        drop(fake);
        report(PrismdReport::Event(event));
    }

    /// Pushes one tick's telemetry to the session.
    pub fn sample(&self, gpus: GpuPerformanceResponse) {
        self.push(DaemonEvent::TelemetrySampled(gpus));
    }

    /// Every request Prism sent on a session, oldest first.
    pub fn requests(&self) -> Vec<DaemonRequest> {
        self.0.lock().unwrap().requests.clone()
    }

    fn set_state(&self, state: DaemonState) {
        let mut fake = self.0.lock().unwrap();
        fake.state = state;
        let report = fake.report.clone();
        drop(fake);
        if let Some(report) = report {
            report(report_for(state));
        }
    }
}

impl PrismdConnector for FakePrismd {
    fn start(&self, report: Box<dyn Fn(PrismdReport) -> bool + Send + Sync>) {
        let report: Report = Arc::from(report);
        let mut fake = self.0.lock().unwrap();
        fake.report = Some(report.clone());
        let state = fake.state;
        drop(fake);
        report(report_for(state));
    }

    fn send(&self, request: DaemonRequest) {
        let mut fake = self.0.lock().unwrap();
        if fake.state.welcomes() {
            fake.requests.push(request);
        }
    }
}

impl DaemonState {
    /// Whether a connecting Prism gets a session.
    fn welcomes(self) -> bool {
        matches!(self, Self::Running | Self::ReadOnly)
    }
}

/// What connecting to a daemon in `state` reports.
fn report_for(state: DaemonState) -> PrismdReport {
    let handshake = contract(include_str!(
        "../../../prism-protocol/tests/contracts/session-handshake.json"
    ));
    match state {
        DaemonState::NotRunning => PrismdReport::NotConnected(NotConnected::Unavailable {
            message: "No such file or directory (os error 2)".into(),
        }),
        DaemonState::Running | DaemonState::ReadOnly => {
            let answer = if state == DaemonState::Running {
                "welcome"
            } else {
                "read_only_welcome"
            };
            let SessionMessage::Response {
                response: prism_protocol::DaemonResponse::Welcome(welcome),
                ..
            } = serde_json::from_value(handshake[answer].clone()).unwrap()
            else {
                panic!("the handshake contract holds a welcome")
            };
            PrismdReport::Connected {
                welcome,
                snapshot: snapshot(),
            }
        }
        DaemonState::Incompatible => {
            let SessionMessage::Response {
                response: prism_protocol::DaemonResponse::Incompatible(incompatible),
                ..
            } = serde_json::from_value(handshake["incompatible"].clone()).unwrap()
            else {
                panic!("the handshake contract holds an incompatible answer")
            };
            PrismdReport::NotConnected(NotConnected::Incompatible(incompatible))
        }
    }
}

/// The snapshot from the shared contract: the simulated tunable GPU.
pub fn snapshot() -> DaemonSnapshot {
    let message: SessionMessage = serde_json::from_value(contract(include_str!(
        "../../../prism-protocol/tests/contracts/daemon-snapshot.json"
    )))
    .unwrap();
    let SessionMessage::Snapshot { snapshot } = message else {
        panic!("the snapshot contract holds a snapshot")
    };
    snapshot
}

fn contract(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}
