//! The fake prismd connection: the same connector interface as the real one, driven by the test
//! on its own thread, with snapshots and events from the shared contract fixtures.

use std::sync::{Arc, Mutex};

use prism::{PrismdConnector, PrismdReport, PrismdService};
use prism_gpu::GpuPerformanceResponse;
use prism_protocol::{DaemonEvent, DaemonRequest, DaemonResponse, DaemonSnapshot, SessionMessage};
use prism_ui::{
    CommandId, Distro, PrismdHost, PrismdInstall, ServiceAction, ServiceOutcome, SessionFailure,
    UnitState,
};

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
    /// The commands Prism sent that the test hasn't answered yet, oldest first.
    unanswered: Vec<CommandId>,
    /// The snapshot a session starts from.
    snapshot: DaemonSnapshot,
}

impl FakePrismd {
    pub fn new(state: DaemonState) -> Self {
        Self(Arc::new(Mutex::new(Fake {
            state,
            report: None,
            requests: Vec::new(),
            unanswered: Vec::new(),
            snapshot: snapshot(),
        })))
    }

    /// Makes the next session start from `snapshot`, such as one with Activity from while Prism
    /// was closed.
    pub fn set_snapshot(&self, snapshot: DaemonSnapshot) {
        self.0.lock().unwrap().snapshot = snapshot;
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

    /// Answers the oldest command Prism sent that isn't answered yet with `response`.
    ///
    /// # Panics
    ///
    /// When there's no command to answer.
    pub fn answer(&self, response: DaemonResponse) {
        let mut fake = self.0.lock().unwrap();
        assert!(!fake.unanswered.is_empty(), "no command to answer");
        let id = fake.unanswered.remove(0);
        let report = fake.report.clone().expect("Prism hasn't connected");
        drop(fake);
        report(PrismdReport::Answered { id, response });
    }

    /// Every request Prism sent on a session, oldest first.
    pub fn requests(&self) -> Vec<DaemonRequest> {
        self.0.lock().unwrap().requests.clone()
    }

    fn set_state(&self, state: DaemonState) {
        let mut fake = self.0.lock().unwrap();
        fake.state = state;
        // A session that ends leaves its commands unanswered.
        fake.unanswered.clear();
        let report = fake.report.clone();
        let snapshot = fake.snapshot.clone();
        drop(fake);
        if let Some(report) = report {
            report(report_for(state, snapshot));
        }
    }
}

impl PrismdConnector for FakePrismd {
    fn start(&self, report: Box<dyn Fn(PrismdReport) -> bool + Send + Sync>) {
        let report: Report = Arc::from(report);
        let mut fake = self.0.lock().unwrap();
        fake.report = Some(report.clone());
        let state = fake.state;
        let snapshot = fake.snapshot.clone();
        drop(fake);
        report(report_for(state, snapshot));
    }

    fn send(&self, request: DaemonRequest) {
        let mut fake = self.0.lock().unwrap();
        if fake.state.welcomes() {
            fake.requests.push(request);
        }
    }

    fn command(&self, id: CommandId, request: DaemonRequest) {
        let mut fake = self.0.lock().unwrap();
        if fake.state.welcomes() {
            fake.requests.push(request);
            fake.unanswered.push(id);
        }
    }
}

impl DaemonState {
    /// Whether a connecting Prism gets a session.
    fn welcomes(self) -> bool {
        matches!(self, Self::Running | Self::ReadOnly)
    }
}

/// What connecting to a daemon in `state`, whose session starts from `snapshot`, reports.
fn report_for(state: DaemonState, snapshot: DaemonSnapshot) -> PrismdReport {
    let handshake = contract(include_str!(
        "../../../prism-protocol/tests/contracts/session-handshake.json"
    ));
    match state {
        DaemonState::NotRunning => PrismdReport::NotConnected(SessionFailure::Unavailable {
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
                snapshot,
                socket: "/run/prism/prismd.sock".into(),
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
            PrismdReport::NotConnected(SessionFailure::Incompatible(incompatible))
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

/// The host as Prism asks it about prismd: systemd natively, or the Flatpak sandbox, on Ubuntu.
/// Enabling or restarting prismd starts the fake daemon. Clones share the same host.
#[derive(Clone)]
pub struct FakeHost(Arc<Mutex<Host>>);

struct Host {
    install: PrismdInstall,
    daemon: FakePrismd,
    /// How the next service action ends, then done.
    next_outcome: Option<ServiceOutcome>,
    runs: Vec<ServiceAction>,
}

impl FakeHost {
    pub fn new(install: PrismdInstall, daemon: FakePrismd) -> Self {
        Self(Arc::new(Mutex::new(Host {
            install,
            daemon,
            next_outcome: None,
            runs: Vec::new(),
        })))
    }

    /// The side Prism holds.
    pub fn service(&self) -> Box<dyn PrismdService> {
        Box::new(self.clone())
    }

    /// Makes the next service action end with `outcome`, rather than done.
    pub fn answer_next_run(&self, outcome: ServiceOutcome) {
        self.0.lock().unwrap().next_outcome = Some(outcome);
    }

    /// Every service action Prism ran, oldest first.
    pub fn runs(&self) -> Vec<ServiceAction> {
        self.0.lock().unwrap().runs.clone()
    }
}

impl PrismdService for FakeHost {
    fn check(&self, checked: Box<dyn FnOnce(PrismdHost) + Send>) {
        let install = self.0.lock().unwrap().install.clone();
        checked(PrismdHost {
            install,
            distro: Distro {
                id: "ubuntu".into(),
                id_like: vec!["debian".into()],
                name: "Ubuntu 24.04 LTS".into(),
            },
            nvidia_driver: None,
        });
    }

    fn run(&self, action: ServiceAction, done: Box<dyn FnOnce(ServiceOutcome) + Send>) {
        let mut host = self.0.lock().unwrap();
        host.runs.push(action);
        let outcome = host.next_outcome.take().unwrap_or(ServiceOutcome::Done);
        let daemon = host.daemon.clone();
        if outcome == ServiceOutcome::Done {
            host.install = PrismdInstall::Native(UnitState::Active);
        }
        drop(host);
        done(outcome.clone());
        if outcome == ServiceOutcome::Done {
            daemon.start_daemon();
        }
    }
}
